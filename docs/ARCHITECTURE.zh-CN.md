# barq 架构

[English](ARCHITECTURE.md)

这是 barq 唯一的架构文档，自顶向下展开：先讲项目为什么存在、整个系统的形状，再按数据流经的顺序逐一讲流水线的各个阶段，然后是贯穿各阶段的问题（并行、Python 边界、验证、性能），最后是计划中的工作和已知缺口。

中英文两个版本的章节编号完全一致，代码注释里写的 `docs/ARCHITECTURE.md §7.3` 在两个文件里指的是同一节。

**以代码为准。** 本文每一条说法都对照过提交 `be1f38c`（分支 `claude/zealous-bardeen-7bz0l5`）上的代码。文中的实测数字都注明了机器和测量条件，它们是观测结果，不是保证。

---

## 目录

0. [约定](#0-约定)
1. [目标与动机](#1-目标与动机)
2. [系统总览](#2-系统总览)
3. [基础层 barq-core 与 barq-model](#3-基础层-barq-core-与-barq-model)
4. [数据层 barq-data 与 barq.data](#4-数据层-barq-data-与-barqdata)
5. [因子表达式 barq-expr](#5-因子表达式-barq-expr)
6. [信号](#6-信号)
7. [撮合内核 barq-exec](#7-撮合内核-barq-exec)
8. [模拟循环 barq-sim](#8-模拟循环-barq-sim)
9. [记录 barq-records](#9-记录-barq-records)
10. [分析 barq-analysis](#10-分析-barq-analysis)
11. [并行](#11-并行)
12. [Python 层 barq-py](#12-python-层-barq-py)
13. [研究层 research/barq_lab](#13-研究层-researchbarq_lab)
14. [验证](#14-验证)
15. [基准测试与性能](#15-基准测试与性能)
16. [开发流程](#16-开发流程)
17. [路线图与已知缺口](#17-路线图与已知缺口)
- [附录 A 许可与代码来源](#附录-a-许可与代码来源)
- [附录 B 术语表](#附录-b-术语表)
- [附录 C 文件索引](#附录-c-文件索引)

---

## 0. 约定

- **行是时间，列是标的**（或"标的 × 参数组合"）。`n_rows` 是 K 线根数，`n_cols` 是列数。
- **列主序存储。** 二维数组是一整块 `f64` 缓冲区，元素 `(row, col)` 位于 `data[col * n_rows + row]`。一列就是一段连续的切片（§3.2）。
- 路径都相对于仓库根目录。`crate::module::item` 指 Rust 条目，`path.rs:function` 指某个文件里的函数。
- "参考实现"指某个行为对齐的对象：撮合、模拟、记录和分析对齐 vectorbt 1.1.0，因子表达式对齐 Qlib。
- "阶段 1"指现有的向量化层。"阶段 2"指计划中的事件驱动层（§2.2、§17.1），目前没有任何实现。

---

## 1. 目标与动机

### 1.1 barq 用来做什么

按优先级：

1. **并行参数扫描。** 成千上万个参数组合作为同一次模拟里互相独立的列运行，铺满所有核。
2. **跨标的或多资产账户的并行回测。** 标的也是列；多标的账户是一组共享现金的相邻列（§8.3）。
3. **大规模因子表达式。** 一套 Qlib 风格的表达式语言（§5）在整块面板上计算 Alpha158 这类因子库，结果不出引擎就能直接进入信号和模拟。

次要目标是：将来可以用 NautilusTrader 的风格写策略——做到源码层面可移植，而不是依赖它的代码（§17.1）。

### 1.2 决定设计的两个发现

**把模拟循环改写成 Rust 并不会更快。** vectorbt 自带一个 Rust 后端（`vectorbt-rust`，约 1.56 万行，PyO3）。它公布的基准测试里，`simulate_from_orders` 相对 Numba 只有 **0.73×–1.15×**：Numba 本来就生成 LLVM 机器码，一个分支密集的标量循环没给 Rust 留下额外可优化的空间。同一个后端在 1000×100 规模的 `diff`、`pct_change`、`fshift`、`bshift` 上能达到 **6–10×**——这些操作可以向量化。同一个后端两种结果，分界线恰好是"能否向量化"。barq 自己的测试框架复现了第一个数字：vectorbt 的 Rust 引擎跑出其 Numba 引擎的 0.74×–0.82×（§15.1）。

**参考的事件驱动引擎在结构上就无法并行扫描。** NautilusTrader 的 `BacktestNode::run()` 是一个串行 `for` 循环——它的回测 crate 里没有 rayon、没有 `par_iter`、没有任何线程创建。它的状态是挂在进程级全局消息总线后面的 `Rc<RefCell<..>>`，所以一个进程只能跑一个引擎，用户要靠启动多个进程来做扫描。每个事件约 1.2–1.9 µs（回放约 86 万事件/秒，跑一个 K 线策略约 54 万/秒）。

所以机会不在于"用 Rust 再做一遍同样的东西"，而在于参考架构已经放弃了的并行能力，再加上一条喂给它的因子流水线。实测结果（§15.1）：单线程下循环比 vectorbt 的 Numba 循环快 2.6×–3.3×，4 线程下快 9.7×–12.1×。差距来自设计选择（列主序扫描、栈上局部状态、`FlexArray` 的标量快速路径、多线程），而不是编译器；各项分别贡献多少，还没有测量。

### 1.3 barq 不是什么

- （目前）不是事件驱动引擎。没有订单簿、没有逐笔数据、没有延迟模型；撮合发生在 K 线上（§2.4）。
- 不是数据供应商。库 crate 从不下载也从不写数据；联网代码只存在于 `examples/`（§4）。
- 不是资金分配或风控系统。下单数量完全由调用方指定（§7.3）；没有自动组合构建。

---

## 2. 系统总览

### 2.1 流水线

```
                    本地 CSV / Parquet / catalog
                                │  barq-data（§4）
                                ▼
            BarPanel：时间 × 标的 的 OHLCV（外加对齐好的辅助序列）
                                │  barq-expr（§5）
                                │  解析 → 编译成依赖图 → 按标的求值
                                ▼
            因子矩阵：每个因子一个 Matrix（时间 × 标的）
                                │  信号规则、延迟、清洗（§6）
                                ▼
       entries/exits 或目标仓位：时间 × （参数 × 标的）列
                                │  barq-sim 调用 barq-exec（§7、§8）
                                │  在互相独立的账户之间并行
                                ▼
       记录：orders、logs → trades、positions、drawdowns（§9）
                                │  barq-analysis（§10）
                                ▼
       每个账户的现金、净值、收益、指标、统计，以及扫描结果汇总
```

每个箭头都是对内存数组的一次函数调用，阶段之间没有 dataframe：因子是 `barq_core::matrix::Matrix`，信号和参数通过 `FlexArray` 借用（§3.3），记录是结构体数组（§9）。

### 2.2 两层，由记录和同一个撮合内核统一

向量化引擎快，是因为它没有事件、没有逐 K 线的对象、没有消息总线；事件驱动引擎逼真，恰恰是因为它有这些。两者不可能在同一条代码路径上兼得：向量化循环每列每根 K 线是纳秒级，事件驱动每个事件是微秒级——差了三个数量级，而这个差距*本身就是*逼真度。

- **A 层（阶段 1，已实现）：** 向量化。输入静态数组，输出记录。参数和标的都是列。
- **B 层（阶段 2，计划中）：** 事件驱动，NautilusTrader 风格的 `Strategy` 对象面对一个模拟交易所，在多次运行之间并行，而不是在一次运行内部并行。

把两层统一起来的是输出，而不是计算。两层写出相同的记录格式（§9），因此分析层只写一次；向量化扫描和事件驱动回放可以逐列比对；用户可以先快速原型、再高保真验证。前提是两层都调用同一个 `barq_exec::execute_order`（§7）：一旦撮合逻辑分叉，A 层就不再能代表 B 层。所以阶段 1 为阶段 2 保留了两条约束：内核可以针对单根 K 线独立调用；记录写入通过 `RecordSink` trait（§9.3）。

一个把两者连起来的观察：vectorbt 的 Rust 后端拒绝接受回调函数的接口（`_engine.py`："Rust engine is not implemented for callback-accepting functions"），所以 `from_order_func` 留在了 Numba。vectorbt 的 `order_func_nb(OrderContext) -> Order` 和 Nautilus 的 `on_bar(&Bar) { submit_order }` 形状相同；在 Rust 里这就是一次单态化的 trait 调用，没有间接开销。因此一个 Nautilus 风格的策略 API 同时也可以作为 `from_order_func` 的原生实现。

### 2.3 Crate 划分

工作区共 13 个 crate（`Cargo.toml` 的 `[workspace] members`）：

| Crate | 职责 | 依赖的工作区 crate |
|---|---|---|
| `barq-core` | `Matrix`、`FlexArray`、列标签、浮点容差、`Error`（§3） | 无（也没有外部依赖） |
| `barq-model` | 撮合枚举、标识符、`BarType`、`OhlcvPanel`、定点 `Price`/`Quantity`（§3.6） | core |
| `barq-generic` | 滚动/扩展/EWM 窗口、平移、填充、交叉（§5.5、§6） | core |
| `barq-signals` | 信号清洗、离场生成、排名、区间（§6.2） | core |
| `barq-indicators` | MA、MSTD、BBANDS、RSI、STOCH、MACD、ATR、OBV 及参数扫描（§6.4） | core、generic |
| `barq-expr` | 因子表达式语言：解析器、依赖图编译器、原生与 Polars 后端、Alpha158/360（§5） | core、generic、model |
| `barq-exec` | 订单撮合内核（§7） | core、model |
| `barq-records` | 记录格式、sink、成交与回撤推导、mapped array（§9） | core、exec、model |
| `barq-sim` | `from_orders` / `from_signals` 循环、止损止盈、分组、并行执行（§8） | core、exec、model、records |
| `barq-analysis` | `Portfolio`、净值/收益链、指标、统计、汇总（§10） | core、generic、model、records、sim |
| `barq-data` | CSV/Parquet/catalog → Arrow → 面板，按时点对齐（§4） | model |
| `barq` | 门面：重新导出所有 crate、`prelude`、`SignalSweep`（§6.1） | 以上全部；`data` feature 引入 barq-data |
| `barq-py` | PyO3 扩展 `barq._barq` 和 Python 包 `barq`（§12） | data、analysis、core、expr、generic、model、records、sim |

从下到上的分层：`core → model → exec → records → sim → analysis`；旁支有 `generic → indicators / expr`、`signals`（只依赖 core）和 `data`（只依赖 model）。`barq-py` 不依赖 `barq-signals`、`barq-indicators` 和 `barq` 门面，它通过 `barq-sim` 间接使用撮合内核。

外部运行时依赖刻意保持很少：

| Crate | 外部依赖 | 原因 |
|---|---|---|
| `barq-expr` | `rayon` | 按标的并行求值因子（§5.4） |
| `barq-expr`（feature `polars`） | `polars` 0.46 | 可选的第二个后端（§5.6） |
| `barq-data` | `arrow-*` 59、`parquet` 59、`csv` | 读取本地数据（§4） |
| `barq-py` | `pyo3` 0.29（abi3-py39）、`numpy` 0.29、arrow `ffi` | Python 边界（§12） |

模拟器本身（`barq-sim`）只用 `std::thread::scope`（§11.2）。

### 2.4 范围决策

- **账户默认互相独立。** 每一列都是一个有自己现金的账户。`cash_sharing=True` 配合 `group_lens` 让相邻列共享一个账户，这样一个参数组合可以持有多个标的，而不同参数组合之间仍然隔离、可以并行（§8.3）。
- **账户内的订单是串行的。** 在一根 K 线内，共享账户按调用顺序依次执行它的各列；`call_seq="auto"` 按估算的订单金额排序，保证先卖后买（§8.3）。
- **指标按账户算，记录按标的存。** 收益和指标从账户净值计算；订单、成交和日志保留各自的标的列（§10）。
- **只支持 K 线。** 撮合发生在 K 线价格上。资金费率、新闻等事件数据可以通过按时点对齐作为因子输入（§4.4）；逐笔和订单簿不在范围内。
- **Rust 引擎，Python 边界。** 动态 schema、字符串和 Python 回调都不进入模拟循环。用户解码器在加载数据时运行（§4.2），早于任何模拟。
- **扫描不复制价格。** 源 OHLCV 保持 `[T, S]`；扫描产生的逻辑列通过 `FlexArray::MappedColumns`（Python 里是 `column_map`）映射到源列，撮合和分析都这样处理（§3.3、§12.3）。

---

## 3. 基础层 barq-core 与 barq-model

### 3.1 列轴

参数和标的共用一条扁平的列轴：

```
col = param_idx * n_symbols + symbol_idx
```

3 个均线窗口（10、20、50）× BTC 和 ETH 得到 6 列：

| col | 0 | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|---|
| window | 10 | 10 | 20 | 20 | 50 | 50 |
| symbol | BTC | ETH | BTC | ETH | BTC | ETH |

- 参数标签**重复**（`aabbcc`，`Level::repeat`），标的标签**平铺**（`abcabc`，`Level::tile`），见 `crates/barq-core/src/columns.rs`。
- **连续性。** 一个参数组合占据相邻的列，所以共享现金的账户是一段连续切片：`group_lens=[2,2,2]` 得到 3 个账户、每个 2 个标的。
- **统一性。** 模拟器只需要知道形状和分组长度，永远不需要知道哪一列对应哪个参数。
- 这个布局是由构造函数保证的约定（`Level::repeat`/`tile`、`barq_indicators::sweep::column_of`、`barq::sweep::SignalSweep`、`barq.strategy.sweep`），运行时不检查。`columns.rs` 里有一个测试（`one_parameter_combination_occupies_contiguous_columns`）锁定它。

**标签。** `Columns { levels, n_cols }` 携带命名的层级。`Label` 是 `Int | Float | Str | Bool`（浮点按比特比较）。`Columns::group_indices(level)` 按首次出现顺序返回 `(label, [cols])`，是按参数汇总（§10.5）的基础：2,000 个不分组的列产出 2,000 个夏普比率，`group_by(columns, "window", values, Reduce::Mean)` 按窗口归约；`best_column` 把最优列报告为 `window=20, symbol=BTC`。`Columns::level` 出错时会列出可用的层级；`new` 拒绝宽度不同的层级。标签放在基础 crate 里，因为因子筛选依赖它。

### 3.2 内存布局

- `barq_core::matrix::Matrix { data: Vec<f64>, n_rows, n_cols }` 是列主序：`column(c)` 不复制地返回一段连续的 `&[f64]`。
- 为什么列主序：列是工作单元。不分组的模拟从第一根 K 线扫到最后一根只看一列；因子一次按一个标的求值（§5.4）；一个线程拿走一段列而不会碰到其他线程的缓存行。共享账户则逐根 K 线推进一组相邻的列。
- `barq_model::OhlcvPanel { open, high, low, close, volume, n_rows, n_cols }` 是结构体数组：每个字段是独立的列主序缓冲区，因为止损引擎和因子输入是分字段读取的。空字段表示"未提供"。
- NumPy 默认是行主序，所以跨越 Python 边界的每个数组都必须是 Fortran 序（§12.2）。测试夹具的生成脚本用 `order="F"` reshape 也是这个原因。

### 3.3 用 `FlexArray` 做广播

价格、手续费、滑点、止损、下单数量和大多数参数都可以是一个标量、每列一个值、每行一个值，或者一个完整矩阵。全部展开需要每个参数 `n_rows × n_cols × 8` 字节——10 万根 K 线 × 2,000 列就是 1.6 GB，而这样的参数有十几个。所以 `barq_core::flex::FlexArray<'a, T>` 只借用传进来的数据：

| 变体 | 含义 | `get(row, col)` 读取 |
|---|---|---|
| `Scalar(T)` | 处处同一个值 | 该值（不访问内存） |
| `PerColumn(&[T])` | 每列一个值 | `values[col]` |
| `PerRow(&[T])` | 每根 K 线一个值 | `values[row]` |
| `MappedColumns { data, n_rows, columns }` | 逻辑列 → 共享矩阵中的源列 | `data[columns[col] * n_rows + row]` |
| `Full { data, n_rows }` | 完整的列主序矩阵 | `data[col * n_rows + row]` |

- `get` 是一个 `#[inline]` 的 match。`varies_by_row()` 让循环把每列不变的量提到 K 线循环之外（§8.1）。
- 带检查的构造函数（`scalar`、`per_column`、`per_row`、`full`、`mapped`）校验长度，不匹配时返回 `Error::FlexLen`。
- `MappedColumns` 是参数扫描便宜的原因：100 个标的上 50 组参数需要 5,000 个逻辑价格列，但内存里只存在 100 个源列。Python 里对应 `column_map`（§12.3），`barq.strategy.sweep` 会自动构造它（§6.3）。

### 3.4 浮点容差层

`crates/barq-core/src/math.rs` 很小，却是整个工作区最重要的数值规则：

- `REL_TOL = 1e-9`、`ABS_TOL = 1e-12`，与参考实现一致。
- `is_close(a, b)` 遵循 PEP 485 的 `isclose`，只有一处刻意的偏离：**NaN 和任何无穷都不"接近"，连 `inf` 与 `inf` 也不。** 无穷在内核里是有意义的哨兵值——`size = +inf` 表示"能买多少买多少"，`price = +inf` 表示"用收盘价"——绝不能被当作普通数值比较。
- `is_close_or_less(a, b)` 是 `is_close || a < b`；`is_less(a, b)` 是 `!is_close && a < b`。能否负担一笔订单都用它们判断，从不直接用 `<=`（§7.5）。
- `add(a, b)` 和 `is_addition_zero` 在 `a + b` 接近零时返回精确的 `0.0`（异号时按量级比较）。内核里每一次余额更新都用 `add`，所以现金和持仓不会残留 `1e-17` 这种尾差。
- 工作区 lint `clippy::float_cmp = "deny"` 禁止对浮点用 `==`；确实需要精确比较的地方带一个局部 `#[allow]` 并写明原因。

### 3.5 错误与拒单的区别

`barq_core::error::Error` 有 5 个变体：`Broadcast`、`FlexLen`、`UnknownLevel`、`LevelWidth` 和 `Invariant(String)`。`Err` 表示某个不变量被破坏了——现金为负、持仓不是有限值、只做多的列持有空头、参数非法——运行无法有意义地继续。

**被拒绝的订单从来不是 `Err`。** 无法成交的订单（没钱、低于最小数量、价格是 NaN）返回一个 `OrderResult`，其 `status` 为 `Ignored` 或 `Rejected`，`status_info` 说明原因（§7.6）。"为什么没成交"是用户问的第一个问题，所以每种原因都是独立的、并且会被记录（§9.4）。

### 3.6 领域模型（barq-model）

- **撮合枚举**（`crates/barq-model/src/enums.rs`）是 `#[repr(i8)]`，判别值由测试锁定，因为记录里以整数存储它们：`SizeType {Amount, Value, Percent, TargetAmount, TargetValue, TargetPercent}`、`Direction {LongOnly, ShortOnly, Both}`、`OrderSide {Buy, Sell}`、`OrderStatus {Filled, Ignored, Rejected}`，以及 14 种原因的 `StatusInfo`（§7.6）。信号流水线枚举（`AccumulationMode`、`ConflictMode`、`DirectionConflictMode`、`OppositeEntryMode`）、止损枚举（`StopEntryPrice`、`StopExitPrice`、`StopExitMode`、`StopUpdateMode`）和记录枚举（`TradeDirection`、`TradeStatus`、`TradesType`）都遵循 vectorbt 的语义。
- **标识符**（`ids.rs`）：`Symbol`、`Venue`（都是 `Arc<str>` 的新类型）和 `InstrumentId { symbol, venue }`，显示为 `SYMBOL.VENUE`。解析时按最后一个点切分，拒绝空白、控制字符和路径分隔符。它们克隆开销小，但还没有做驻留（interning）（§17.4）。
- **`BarType`**（`bar.rs`）格式为 `SYMBOL.VENUE-STEP-AGGREGATION-PRICE-SOURCE`，例如 `BTCUSDT.BINANCE-1-MINUTE-LAST-EXTERNAL`。解析用 `rsplitn(5, '-')`，所以标的名里可以有 `-`。它也是数据 catalog 的键格式（§4.1）。
- **定点数值**（`types.rs`）：`Price { raw: i64, precision }` 和 `Quantity { raw: u64, precision }`，9 位小数（`PRECISION = 9`），沿用 NautilusTrader 的形状。raw 类型是别名，所以将来的 16 位小数 `i128` 版本只是换一个类型，不需要重新设计。它们为阶段 2 而存在；向量化层全程使用 `f64`。
- 两条血统是刻意的：撮合语义沿用 vectorbt，数值与标识符类型沿用 NautilusTrader（§17.1）。

---

## 4. 数据层 barq-data 与 barq.data

`crates/barq-data/src/lib.rs` 把本地文件变成面板。它从不下载也从不写入；下载和缓存代码在 `examples/` 里（`binance.py`、`fetch.py`、`download_binance_vision.py`）。

### 4.1 Catalog 目录布局

```
<root>/data/<name>/<key>/*.csv | *.parquet
```

- `name` 是数据集（`bars`、`funding`、`news` 等）；对 K 线来说 `key` 是 `BarType` 字符串（`BTCUSDT.BINANCE-1-HOUR-LAST-EXTERNAL`），其他数据是 `InstrumentId`。两者都必须是单个路径分量。
- `barq_data::catalog(root, name, keys)` 列出这些文件并排序。Python 默认的 root 是 `~/.cache/barq/catalog`；`examples/download_binance_vision.py` 用 Binance 公开归档的按月 Parquet 文件填充它。

### 4.2 Schema 与加载

`DataSchema { name, input, output, event_time, available_time, instrument }` 描述一个数据集：

- `input`：从文件读取的 Arrow schema（即投影）。
- `output`：解码之后的 schema。`event_time` 和 `available_time` 必须是 `UInt64` 的 UTC 纳秒。`available_time` 是该值变得可知的时间，所有过滤和连接都用它——这正是防止晚发布数据造成前视偏差的关键。
- `instrument`：`Source`（由 key 指定标的）、`Columns { symbol, venue }`（每行自带标的列，`Utf8`）或 `Global`（对所有标的生效）。

`load(schema, items, start, end, decode)` 对每个文件：

1. 按每批 65,536 行流式读取 `RecordBatch`（Parquet 用投影掩码；CSV 按 `input` 中的类型读取，读不出来时退回 `Utf8`）；
2. 投影到 `input`，把每一批交给调用方的 `decode` 闭包——可以是原生 Rust，也可以是通过 Arrow C Data Interface 调用的 Python 回调（§12.4）；
3. 投影到 `output`，检查时间列没有空值、每行的标的与 key 中的 `BarType` 一致；
4. 保留满足 `start <= available_time < end` 的行。

结果是一个 `Dataset { schema, keys, batches }`。

### 4.3 从 K 线到面板

`Dataset::prepare()` 构造 `BarPanel { time, bar_types, values: OhlcvPanel }`：

- 每个 key 都必须能解析为 `BarType`；OHLCV 列必须是 `Float64`；
- 时间轴是所有 `available_time` 的有序并集；
- 每个标的的 K 线按列主序写到 `col * n_rows + row`；缺失的 K 线是 `NaN`；重复的 K 线是错误。

### 4.4 辅助数据的按时点对齐

`Dataset::align(time, instruments, field, window, empty)` 把任意数值字段连接到一条目标时间轴上（通常就是 K 线面板的时间轴）：

- 字段被转成 `f64`，按标的分组（`Global` 数据用全局键），按 `available_time` 排序。
- **不带窗口**时是 as-of 连接：时刻 `t` 取 `available_time <= t` 的最后一个值。
- **带 `(width, agg)`** 时在窗口 `(t - width, t]` 上用 `Sum`、`Count` 或 `Mean` 聚合。求和与均值跳过非有限值；窗口内没有有限值时用 `empty` 填充。
- 输出是列主序的 `time.len() × instruments.len()`；全局事件广播给每个标的。

Python 里对应 `Dataset.asof(panel, field=…)` 和 `Dataset.window(panel, field=…, window="30m", agg="mean")`。`examples/multi_data.py` 把资金费率（as-of）和新闻（窗口均值）对齐到 1 分钟 K 线上，并用在一个入场过滤条件里。

### 4.5 Python 数据 API

`crates/barq-py/python/barq/data.py` 封装了 Rust 层：

- `DataSchema(name, input=pa.schema, output=pa.schema, decode=fn, event_time, available_time, instrument)`；
- `files({key: path | [paths]}, schema=…)` 和 `Catalog(root, schemas=[…]).source(name, keys=…)` → `Source` → `.load(start, end)` → `Dataset`，或 `.prepare()` → `BarPanel`；
- `BarPanel` 提供 `time`、`bar_types`、`instruments`、`open … volume`（`(T, S)` 的 Fortran 序数组）、`shape`、`coverage()`。

加载、`prepare` 和 `align` 会释放 GIL（§12.5）；Python 解码器每处理一批会重新获取 GIL。

---

## 5. 因子表达式 barq-expr

`barq-expr` 从命名的表达式计算因子矩阵：

```rust
let factors = FactorSet::compile([("trend", "$close / Mean($close, 20) - 1")])?;
let values = factors.evaluate(&FactorInputs::from_ohlcv(&panel)?)?;
let trend: &Matrix = values.get("trend").unwrap();   // 时间 × 标的
```

因子是信息，不是决策。把它变成订单——阈值、排名、延迟一根 K 线、清洗——是 §6 的工作。因子矩阵也可以导出到 crate 之外做模型训练；模型的输出不过是另一个打分矩阵。

### 5.1 语言

- **字段**写作 `$name`，统一转小写：`$open $high $low $close $volume` 来自 `FactorInputs::from_ohlcv`；其他字段（`$vwap`、资金费率等）用 `FactorInputs::with_field(name, &[f64])` 添加，长度必须是 `n_rows × n_cols`。缺失的字段在求值时报错。
- **数字**是十进制或科学计数法字面量。
- **中缀运算符**，按优先级从低到高：`|`、`&`、比较（`> < >= <= == !=`）、`+ -`、`* /`、一元 `+ -`、`**`（右结合）。比较和布尔运算返回 `1.0`/`0.0`。

函数（参数个数由 `parse.rs:validate` 检查，语义见 `graph.rs` 和 `eval.rs`）：

| 类别 | 函数 | 说明 |
|---|---|---|
| 逐元素 | `Abs Sign Log Not`（1 个参数）；`Add Sub Mul Div Power Gt Ge Lt Le Eq Ne And Or`（2 个）；`Greater Less`（2 个）；`If(cond, a, b)` | `Greater`/`Less` 是逐元素取大/取小，任一侧为 NaN 时返回 NaN；函数写法和运算符写法是同一个节点 |
| 时间平移 | `Ref(x, n)`、`Delta(x, n)` | `Ref` 按行平移（不规则日历上也一样）；`n` 为负时读取未来，由 `FactorSet::uses_future()` 报告；`Delta` 要求 `n >= 0` |
| 平滑 | `EMA(x, p)` | `p < 1` 是 alpha；`p >= 1` 是 span（`alpha = 2/(p+1)`）；`p = 0` 是扩展 EMA |
| 滚动 `(x, N)` | `Mean Sum Std Var Max Min Med Mad Count Skew Kurt WMA Rank IdxMax IdxMin Slope Rsquare Resi` | `Rank` 是最新值在它自己的滚动窗口里的百分位（时间序列上的，不是横截面）；`IdxMax`/`IdxMin` 返回从 1 开始的位置；`Slope/Rsquare/Resi` 对位置 `1..=N` 做回归 |
| 滚动 `(x, N, q)` | `Quantile` | `q` 在 `0..=1`，线性插值 |
| 成对 `(x, y, N)` | `Corr Cov` | |

窗口规则：`min_periods = 1`（窗口里至少有一个观测就产生值），`N = 0` 表示全部历史（扩展窗口），缺失观测保持 `NaN`。窗口长度、偏移、分位数和 EMA 参数必须是数字字面量。

**因子库。** `alpha158()` 和 `alpha360()` 编译 Qlib 的 `Alpha158DL` 和 `Alpha360DL` 特征集。两者都需要 `open/high/low/close/volume` 外加显式提供的 `vwap` 字段。它们返回原始特征；Qlib `DataHandlerLP` 的标准化、填充和标签不在范围内。

**目前不支持：** 横截面算子（`CSRank`、`CSZScore`、中性化）、跨标的引用、掩码和重采样（§5.7）。

### 5.2 解析

`crates/barq-expr/src/parse.rs` 对源码做词法分析（最多 16,384 字节），用 Pratt 解析器（最大嵌套深度 128）解析成 `Expression { Number | Field | Unary | Binary | Call }`。`validate` 拒绝未知函数和错误的参数个数。`FactorSet::compile` 拒绝空的或重复的因子名以及空的因子集。`FactorSet` 保留 `Expression`，供 `required_fields()`、`uses_future()` 和 Polars 后端使用。

### 5.3 编译成依赖图

`FactorSet::compile` 接着为整个因子集构建一个 `graph::Graph`（`crates/barq-expr/src/graph.rs`）：

1. **哈希合并（hash-consing）。** 每个节点按 `(Op, [输入节点 id])` 去重。被许多因子共享的子表达式——`Ref($close, 1)` 在 Alpha158 里出现了几十次——只成为一个节点；同一件事的不同写法也会合并（`a+b` 与 `Add(a, b)`、`+x` 与 `x`）。
2. **编译期校验。** 字面量参数被解析成带类型的运算符：`Ref(i64)`、`Delta(usize)`、`Ema(Option<alpha>)`、`Rolling(kind, Window)`、`Pair { corr, window }`。小数或负数窗口、超出 `0..=1` 的分位数、负的 EMA 参数、负的 `Delta` 偏移、非字面量参数都在这里报错，错误信息会写出因子名。`Window::All` 代表 `N = 0`。
3. **常量折叠。** 操作数全是字面量的逐元素运算直接变成字面量（`-(1+2)*3` → `-9`）。
4. **依赖顺序。** 节点总是追加在其输入之后，所以节点列表本身就是合法的求值顺序。
5. **暂存槽位调度**（`Graph::schedule`）。每个需要计算的节点分到一个暂存槽位。槽位在最后一个读取它的节点之后释放，供后面的节点复用；一个节点不会分到自己某个输入的槽位。字段和字面量不占槽位（原地读取或按标量广播）。

Alpha158 编译成 357 个节点，只需要 18 个槽位。因此一个标的的整组因子，不管有多少个因子，都只用 18 条序列的暂存空间来计算。

### 5.4 原生求值

`FactorSet::evaluate`（`NativeBackend`）执行 `eval.rs:evaluate`：

1. **解析字段。** 查找依赖图读取的每个字段；缺失的字段在开始任何计算之前就报错。下面的并行部分没有出错路径。
2. **分配输出。** 每个因子一个 `n_rows × n_cols` 缓冲区，逐个单独分配，这样内存页由写入它们的线程惰性清零（如果克隆同一个清零缓冲区，在 2520×200×158 规模上串行要花 350 ms）。
3. **按列切分。** 每个输出切成按列的 `&mut [f64]` 切片，并重新组织成"任务 `c` 拥有每个输出的第 `c` 列"——写入互不重叠，无需加锁，也无需 `unsafe`。
4. **按标的并行求值。** `columns.into_par_iter().enumerate().for_each_init(…)`（rayon）。每个工作线程有自己的暂存区（`slots` 条长度为 `n_rows` 的序列），在它处理的各列之间复用。对一列，`run_column` 按顺序遍历节点：
   - 字段是调用方输入的一段切片（不复制）；字面量是标量；
   - 逐元素节点按"运算符 × 操作数形态"各跑一个紧凑循环（`binary_series` 在循环外分派运算符）；
   - 时间序列节点调用 §5.5 的计算核心；
   - 如果某个节点是某个因子的输出，就把它的序列复制到该因子的对应列（多个因子可以共享同一个节点）。
5. **组装** `FactorValues`（因子名加每个因子一个 `Matrix`）。

结果是：并行发生在标的之间，每次求值只有一个并行区域。每个中间结果只是一个标的的序列（2,520 行时约 20 KB），而不是整块面板，所以内存峰值基本就是输出本身。列数少于核数的面板用不满所有核（§11.1）。有一个测试（`each_symbol_evaluates_exactly_as_it_would_alone`）检查多标的求值的每一列都与单独对该标的求值的结果逐位相同。

### 5.5 计算核心

| 算子 | 实现 | 每行开销 |
|---|---|---|
| `Mean` | 前缀和，`barq_generic::rolling::rolling_mean_1d` | O(1) |
| `Max Min` | 单调双端队列，`barq_generic::rolling` | 均摊 O(1) |
| `Sum Std Var Skew Kurt WMA Slope Rsquare Resi Corr Cov` | 增量累加器，`window.rs` | O(1) |
| `Quantile Med Rank Mad` | 按排序位置建的树状数组（Fenwick tree），`order.rs` | O(log n) |
| `IdxMax IdxMin` | 单调队列，`order.rs:rolling_arg_extreme` | 均摊 O(1) |
| `Count` | `rolling_apply_1d` | O(window) |
| `EMA` | 调整后的递推；扩展形式 | O(1)；`EMA(x, 0)` 每行 O(n) |

**增量累加器**（`window.rs`）加上进入窗口的行、减去离开窗口的行。有三项措施保证数值上靠得住：

- **定期重建。** 每隔 `window` 行从窗口数据重新计算累加和，所以加减产生的舍入误差最多只会累积两个窗口的长度。重建每 `window` 行花一次 O(window)，总开销仍是 O(n)。
- **平移求和。** 数值以某个平移量（空窗口时推入的第一个值）为基准求和，所以价格在 100 附近时，方差不会变成两个大平方数之差。
- **精确的常数窗口。** Qlib 参考实现有精确的零值判断（`Corr` 的 `va == 0`、`Rsquare` 的 `yvar <= 0`）。对完全相同的值做累加会留下舍入残差而不是零，所以 `Moments` 会统计末尾连续相同的观测，当整个窗口都是常数时报告精确为零的方差。

`NaN` 行被跳过：它们占一个窗口位置，但不参与求和。

**顺序统计量**（`order.rs`）对每一列只排一次序，给每一行一个固定的排序位置，并在这些位置上维护一个树状数组，标记哪些行在窗口里。进出窗口是单点更新，第 k 小的值是一次树上下降，"比 x 小的有多少个"是一次前缀查询。计数和位置都是整数，所以 `Quantile`、`Med` 和 `Rank` 与对窗口排序的结果逐位相同。`Mad` 还维护部分和，并在常数窗口上报告精确的零。

**精确性。** 单元测试在压力序列（NaN 段、全 NaN 段、常数段、大水平小波动、噪声）上，把每个增量计算核心与逐窗口完整重算的结果比较，窗口长度从 1 到超过序列长度，相对容差 `1e-9`（`incremental_windows_match_a_full_rescan`、`incremental_pairs_match_a_full_rescan`）。Qlib 测试夹具（§14.3）在 `1e-6`（`CORR*`/`CORD*` 为 `1e-4`）上锁定最终结果。

### 5.6 Polars 后端

开启 `polars` feature 后，`set.evaluate_with(&inputs, &PolarsBackend)` 用 Polars 计算同一个因子集（`crates/barq-expr/src/polars_backend.rs`）。后端总是显式选择；`ComputeBackend` 是扩展点。

- 面板被转换成带 `__barq_expr_symbol` 列的长格式 `DataFrame`。每个因子的 `Expression`（不是依赖图）被编译成 Polars 惰性表达式，以 `.over(symbol)` 求值，每批 64 个因子。
- 标准滚动运算映射到 Polars 原生的滚动函数；Qlib 特有的（`Rank`、`IdxMax/IdxMin`、`Slope/Rsquare/Resi`、`Skew/Kurt`、`Mad`、`Count`、`WMA`）用 `rolling_map` 调用参考的逐窗口重算实现。

它比原生后端慢得多（在本工作区测量，取 3 次最好成绩，4 线程）：

| 面板 | 负载 | Native | Polars | Polars / Native |
|---|---|---|---|---|
| 1024 × 16 | 4 个逐元素因子 | 0.3 ms | 5.0 ms | 15× |
| 2520 × 200 | 6 个滚动因子（窗口 20） | 31 ms | 102 ms | 3.3× |
| 2520 × 200 | Alpha158 | 0.60 s | 6.2 s | 10× |
| 2520 × 200 | Alpha360 | 0.41 s | 2.7 s | 6.7× |
| 2520 × 500 | Alpha158 | 1.5 s | 19.4 s | 13× |

开销来自：每个算子都要 `.over(symbol)` 重新分组；原生计算核心是 O(1) 或 O(log n) 的地方它用 O(window) 的 UDF；批次边界阻止了共享；以及 `Option<f64>` 转换。这个后端目前的价值在于作为独立实现做交叉验证：Qlib 测试夹具在两个后端上都会跑。

### 5.7 内存、限制与计划

在开发用的 4 核容器上测量（有噪声；相对比较是在同一时段内交替运行新旧构建得到的）：

| Alpha158 | 结果 |
|---|---|
| 2520 × 200，4 线程 | 约 0.8–1.0 s，取决于机器负载 |
| 2520 × 16 | 约 75 ms（依赖图求值器之前是 160 ms） |
| 2520 × 500 的内存峰值 | 约 1.6 GB，几乎全部是 158 个输出矩阵 |

这个设计带来两个限制：

- **所有输出同时驻留内存。** 全 A 股面板（5,000 个标的 × 2,520 天）光 Alpha158 的输出就要约 16 GB。计划：按因子分批求值，把每一批交给调用方提供的 sink 或 Arrow/Parquet 写入器（§17.2）。
- **横截面算子需要分阶段。** 按标的并行求值只有在每个算子都只读取单个标的序列时才成立。`CSRank` 这类算子会在这些节点处把图切开：阶段内按标的并行，阶段之间对所有标的做一次横截面计算（§17.2）。

---

## 6. 信号

信号把因子矩阵（或任何其他数据）变成扫描列轴上逐 K 线的入场/离场布尔值，供 `from_signals` 使用（§8.2）。

**最关键的一条规则：延迟。** 用第 `t` 根 K 线收盘价算出的因子，要等第 `t` 根收盘之后才知道。在第 `t` 根 K 线上用它交易就是前视偏差。下面每个工具要么默认延迟，要么把延迟作为显式参数。`crates/barq/tests/factor_path.rs` 展示了手工路径：因子 → 延迟一根 K 线 → 阈值 → `clean_enex` → `Portfolio::from_signals`。

### 6.1 `SignalSweep`（Rust）

`barq::sweep::SignalSweep::from_factor((name, &Matrix), symbols, cases, delay, rule)`（`crates/barq/src/sweep.rs`）：

- 对每个 case × 标的计算 `rule(factor[row - delay, symbol], &params) -> (entry, exit)`；
- 前 `delay` 行和非有限的分数不产生信号；信号不做清洗（由模拟器的冲突规则处理）；
- 列带有层级 `factor` 和 `case`（重复）以及 `symbol`（平铺）：`col = case_idx * n_symbols + symbol_idx`；
- `expand(&close)` 按 case 重复收盘价，`args(&swept_close)` 返回一个已填好形状、收盘价、entries 和 exits 的 `FromSignals`——其余字段由调用方用结构体更新语法补上。

`crates/barq/examples/factor_sweep.rs` 跑的是：动量因子 → 2 个 case × 3 个标的 → 用 `available_parallelism()` 个线程调用 `Portfolio::from_signals`。

### 6.2 barq-signals

面向扁平列主序 `&[bool]` 的工具（`crates/barq-signals/src`）：

- `clean_enex(entries, exits, n_rows, n_cols, FirstSignal)` 生成严格交替的入场/离场序列；同一根 K 线同时有入场和离场时两者都不产生。
- `generate_exits`、`exit_after(entries, bars, …)`、`exit_on(entries, condition, …)` 按选定的 `ExitWindow` 从入场生成离场；`generate_alternating`。
- `signal_rank`、`partition_rank`、`nth_index`、`normalized_average_index`；`partition_ranges`、`between_ranges`、`between_partition_ranges` 生成 `RangeRecords`。
- 刻意没有提供随机信号生成器。

### 6.3 Python 函数式策略

`crates/barq-py/python/barq/strategy.py`：

```python
@barq.strategy
def ma_cross(data, fast, slow):
    f = barq.factors(data, fast=f"Mean($close, {fast})", slow=f"Mean($close, {slow})")
    return barq.crossed_above(f["fast"], f["slow"]), barq.crossed_above(f["slow"], f["fast"])

pf = ma_cross.sweep(panel, params=[{"fast": 5, "slow": 20}, …], threads=4)
```

- `factors(data, **expressions)` 编译表达式，**拒绝读取未来数据的表达式**（`uses_future`），从 mapping 或 `BarPanel` 取出所需字段并求值。
- `.sweep(data, params, signal_delay=1, **options)` 对每组参数调用一次函数，检查布尔数组形状，把信号延迟 `signal_delay` 根 K 线，拼接各列（`param_idx * S + symbol`），传入 `column_map = tile(arange(S), n_params)` 使价格永不被复制，为止损转发 OHLC，`cash_sharing=True` 时设置 `group_lens = [S] * n_params`（每组参数一个共享账户），然后调用 `from_signals`。
- `.run(data, params=None)` 是只有一组参数的扫描。

### 6.4 技术指标

`barq-indicators` 在参数网格上计算 `ma`、`mstd`、`bbands`、`rsi`、`stoch`、`macd`、`true_range`/`atr` 和 `obv`。`sweep_columns` 与 `column_of(comb, input_col, n_input) = comb * n_input + input_col` 让输出采用 §3.1 的列布局，每个指标内部的缓存让共享的中间结果只算一次。技术指标只在 Rust 中提供，单线程。

---

## 7. 撮合内核 barq-exec

`barq_exec::execute_order(state, order, reject_draw)`（`crates/barq-exec/src/kernel.rs`）在一根 K 线上针对一个账户状态执行一笔订单。它是引擎的语义核心：两个模拟循环都调用它，阶段 2 也会调用它（§2.2）。

### 7.1 状态与请求

- `ExecuteOrderState { cash, position, debt, free_cash }`——`position` 带符号（负数为空头）。
- **`debt` 与 `free_cash`。** `debt` 是未平空头按平均开仓价计算的卖出所得。开空时 `free_cash` 变化 `proceeds - 2 * short_value`：卖出所得记入，但冻结两倍空头市值作为保证金。平空时通过 `avg_entry_price = debt / |position|` 按比例解冻。这个冻结量与路径有关，无法只从现金和持仓推出——所以需要显式的状态。
- `ProcessOrderState` 另外加入 `val_price`、`value`，以及循环使用的订单和日志计数器（`oidx`、`lidx`）。
- `Order { size, price, size_type, direction, fees, fixed_fees, slippage, min_size, max_size, size_granularity, reject_prob, lock_cash, allow_partial, raise_reject, log }`，默认值 `size = +inf`、`price = +inf`、`Amount`、`Both`、`max_size = +inf`、`size_granularity = NaN`、`allow_partial = true`、`min_size = 0.0`。（模拟循环把 `min_size` 默认设为 `1e-8`，见 §8.4。）
- `OrderResult { size, price, fees, side, status, status_info }`。

### 7.2 校验

1. **归零。** `cash`、`position`、`debt`、`free_cash`、`val_price` 和 `value` 如果 `is_close` 于零，就被置为精确的零（§3.4）。
2. **校验前先忽略：** size 为 NaN → `Ignored/SizeNaN`；price 为 NaN → `Ignored/PriceNaN`。
3. **K 线校验**（`validate_bar`）：现金有限且 `>= 0`，持仓有限，负债有限且 `>= 0`，可用现金不是 NaN，价格有限且 `> 0`，只做多的列不能持有空头，只做空的列不能持有多头。
4. **参数校验**（`param_violation`）：手续费和固定手续费有限，滑点 `>= 0`，`min_size >= 0`，`max_size > 0`，粒度为 NaN 或有限且 `> 0`，`reject_prob` 在 `[0, 1]`。

任何违反都返回 `Err(Error::Invariant)`。`ValidatedOrder` 缓存参数部分的校验结果，让循环对每列不变的参数只校验一次，之后只替换逐 K 线的字段（`set_bar`）。内核注释里测得的收益是 12–15%；`benchmarks/README.md` 里的收益归因记录了稀疏负载 +30%、密集负载 +22%。

### 7.3 数量换算链

下单数量按固定顺序归一成一个带符号的数量：

1. `ShortOnly` 把 `size` 取反。
2. `TargetPercent × value → TargetValue`。value 为 NaN → `Ignored/ValueNaN`；value `<= 0` → `Rejected/ValueZeroNeg`。
3. `Value / val_price → Amount`，`TargetValue / val_price → TargetAmount`。`val_price` 为无穷或非正数是错误；为 NaN → `Ignored/ValPriceNaN`。
4. `TargetAmount - position → Amount`。
5. 在 `ShortOnly`/`Both` 下 `-inf` 变成 `Percent(-1)`。
6. `Percent` 变成 `percent = |size|`、`size = ±inf`、类型 `Amount`。
7. 按符号分派：`size > 0` → 买；否则（包括 0 和 NaN）→ 卖，与参考实现一致。

### 7.4 买入与卖出

**买入**（`kernel.rs:buy`）：

- `adj_price = price * (1 + slippage)`。
- 现金上限是 `cash`；开启 `lock_cash` 时则是基于 `free_cash` 的闭式解（三个分支），同时覆盖正在平掉的空头，再以 `percent` 封顶。
- `ShortOnly` 把数量限制在 `-position`（只能平空）。
- 手续费计入现金上限之内：`req_cash = size * adj_price`，`fees_paid = req_cash * fees + fixed_fees`。如果 `is_close_or_less(req_cash + fees_paid, cash_limit)` 不成立，就反解出能负担的名义金额 `(cash_limit - fixed_fees) / (1 + fees)`（`<= 0` 时为 `CantCoverFees`），并在调整数量后再次应用 `size_granularity`——粒度要应用两次。
- 然后是 `max_size`（拒绝，或在 `allow_partial` 时截断）、`SizeZero`、`MinSizeNotReached`（`is_less`）和 `PartialFill`。
- 平空按比例释放负债，并把 `2 * debt_diff` 加回 `free_cash`。

**卖出**（`kernel.rs:sell`）：

- `adj_price = price * (1 - slippage)`；`LongOnly` 把数量限制在持仓以内。
- 开启 `lock_cash` 或 percent 为无穷时，最大空头受 `free_cash` 加多头卖出所得约束（`NoCashShort`、`CantCoverFees`）。
- `max_size`、`NoOpenPosition`、粒度、`SizeZero`、`MinSizeNotReached`、`PartialFill`，以及扣除手续费后所得为负时的 `CantCoverFees`。
- 新开空头使 `debt += short_value`，并冻结 `2 * short_value` 的 `free_cash`。

### 7.5 为什么容差在这里很重要

能否负担由 `is_close_or_less(total_req_cash, cash_limit)` 判断，而不是 `<=`。两者只在 `1e-9` 的区间内有差别，一个把前者替换成后者的变异最初竟然没被测试发现。`size_granularity` 会把这个看不见的差别放大成整整一个单位（成交 5.0 还是 4.0），所以现在有在边界上用 `math.nextafter` 构造的测试夹具来捕捉它（§14.4）。

### 7.6 结果与拒单原因

`OrderStatus` 是 `Filled`、`Ignored` 或 `Rejected`。`StatusInfo` 有 14 种原因：`SizeNaN`、`PriceNaN`、`ValPriceNaN`、`ValueNaN`、`ValueZeroNeg`、`SizeZero`、`NoCashShort`、`NoCashLong`、`NoOpenPosition`、`MaxSizeExceeded`、`RandomEvent`、`CantCoverFees`、`MinSizeNotReached`、`PartialFill`。开启 `raise_reject` 时，拒单在循环里变成错误。

**随机拒单**（`reject_prob`）由内核支持，采用反转的随机数生成方式：均匀分布的随机数由调用方提供，这样并行运行仍是确定性的。随机数在计算出成交之后才比较；命中时返回 `Rejected/RandomEvent` 并保持原状态。目前两个模拟循环都还没有接上它（§17.4）。

---

## 8. 模拟循环 barq-sim

`barq-sim` 在整块面板上运行撮合内核。有两个循环——订单和信号——各自都有普通、带日志、串行和并行版本。

### 8.1 `from_orders`

`FromOrders<'a>`（`crates/barq-sim/src/from_orders.rs`）持有 `n_rows`、`n_cols`，每个输入都是 `FlexArray`：`init_cash`、`close`、`size`、`price`、`size_type`、`direction`、`fees`、`fixed_fees`、`slippage`、`min_size`、`max_size`、`size_granularity`、`lock_cash`、`allow_partial`、`raise_reject`、`val_price`、`log`，另有 `ffill_val_price`、`update_value`、`groups` 和 `call_seq`。一个元素就是一个下单请求（size 为 NaN 表示不下单）。

对每个账户组（`simulate_group`）：

1. **每列一次：** 用该列的参数构造一个 `ValidatedOrder`；只有当某个参数随行变化时才逐根 K 线重建。
2. **逐 K 线，第一遍——价格。** 为组内每一列确定下单价格（`+inf` → 本根收盘价，`-inf` → 上一根收盘价）和估值价格（`+inf` → 下单价格，`-inf` → 上一根收盘价，开启 `ffill_val_price` 时向前填充）。
3. **组净值。** 对共享组，在 K 线开盘时只计算一次账户净值：现金加 `Σ position × last_val_price`。这根 K 线内的每笔订单都以这个开盘净值计算数量；它在整根 K 线内传递，不在每次成交后重新求和。
4. **调用顺序。** 确定这根 K 线的执行顺序（§8.3）。
5. **第二遍——执行。** 通过 `execute_validated` 依次执行每一列；开启 `update_value` 时每次成交后重新估值。把成交推入记录 sink；如果设置了 `log`，再推入一条日志（§9）。

入口：`simulate_from_orders`、`simulate_from_orders_logged`、`simulate_from_orders_parallel(args, n_threads)`、`simulate_from_orders_logged_parallel`。`Portfolio::from_orders(&args, threads)` 调用其中之一并包装记录（§10）。

### 8.2 `from_signals`

`FromSignals<'a>` 增加了 OHLC、信号、冲突模式和止损止盈。默认值（`FromSignals::default()`）：`init_cash = 100`、`LongOnly`、`size = +inf`、`Amount`、`price = +inf`、`min_size = 1e-8`、`accumulate = Disabled`、所有冲突模式为 `Ignore`、`upon_opposite_entry = ReverseReduce`、止损止盈关闭（`NaN`）、`stop_entry_price = Close`、`stop_exit_price = StopLimit`、`upon_stop_exit = Close`、`upon_stop_update = Override`。

**两种信号约定**，由 `read_signals` 取并：

- `entries`/`exits` + `direction`。`Both` 模式下离场信号就是开空信号。
- 显式的 `long_entries`、`long_exits`、`short_entries`、`short_exits`。

**逐 K 线、逐列**（`prepare_order`，在组内任何成交之前对每一列都先运行）：

1. 确定下单价格（`-inf` 表示本根开盘价，没有则退回上一根收盘价）和估值价格。
2. **止损止盈**（如果已布置）：从收盘价/开盘价补全缺失的开高低，然后先检查止损、再检查止盈，调用 `stop_price(position, anchor, stop, open, low, high, hit_below)`：
   - 如果开盘价已经越过触发价（跳空），按开盘价成交；
   - 否则如果本根 K 线的区间触及触发价，按触发价成交；
   - 否则不触发。
   跟踪止损在检查**之后**才把锚点更新为本根最高价（多头）或最低价（空头），所以一根 K 线不会触发由它自己带来的跟踪移动。
3. **信号。** 如果止损止盈触发，`Signals::from_stop` 替换本根的信号；成交价按 `StopExitPrice` 确定（`StopLimit` 同时把滑点置零），离场方式按 `StopExitMode`（平仓、减仓、反手）。否则调用 `read_signals`，只要有入场信号，就依次执行 `resolve_signal_conflict`（先多头后空头）、`resolve_dir_conflict`、`resolve_opposite_entry`。
4. **数量。** `signals_to_size(position, signals, size, size_type, accumulate, val_price)`。只允许 `Amount`、`Value` 和 `Percent`；负数数量是错误。
5. 构造一个 `ValidatedOrder`，数量为零则跳过。

之后的执行与 §8.1 相同（组净值、调用顺序、`execute_validated`）。每次成交后重新布置止损止盈：空仓 → 清除；开仓或反手 → 按新开仓价重新设锚（按 `StopEntryPrice`）；加仓 → 按 `StopUpdateMode`（`Keep`、`Override`、`OverrideNaN`）重新设锚。

`crates/barq-sim/src/signals.rs` 里的各个阶段都是纯函数，并且单独做了穷举测试（§14.3）。

### 8.3 分组、共享现金与调用顺序

- `Groups { lens, starts, of_col }`（`groups.rs`）。`Groups::ungrouped(n)` 是默认值（每列一个账户）。`Groups::from_lens(lens, n_cols)` 把相邻的列分组，并校验长度之和覆盖所有列。
- 一个组的现金取自它第一列的 `init_cash`。
- `CallSeq` 决定一根 K 线内的执行顺序：`Default`（按布局顺序）、`Auto`（按 `approx_order_value` 升序做稳定插入排序，NaN 排最后——先卖后买，这样卖出释放的现金可以在同一根 K 线内用于买入），或 `Explicit(&[usize])`。
- 共享账户本质上是串行的。并行单位变成组（§11.2）；一个持有所有标的的组合只是一个单位，只能在一个线程上跑。
- 共享组的成交按执行顺序输出；分析层在推导成交之前会对自己的副本按 `(col, idx)` 排序（§10.1）。

### 8.4 默认值、日志与 sink

- `barq_sim::DEFAULT_MIN_SIZE = 1e-8`，与参考实现一致。内核的 `Order::default().min_size` 是 `0.0`；这个差别正是在公开日志之后发现的（§9.4）。
- 带日志的版本对每一笔*被考虑*的订单写一行日志——`from_orders` 是每个元素，`from_signals` 是每根信号换算出非零数量的 K 线（参考实现的规则，逐行锁定）。日志默认关闭，因为一行日志约是一条订单记录的五倍宽，而且写入次数多得多。有一个测试断言打开日志不会改变订单结果。
- 两个循环都对 `SinkBuffer`（§9.3）泛型，所以普通版本和带日志版本共享同一份实现。

---

## 9. 记录 barq-records

记录是模拟与分析之间、以及两层之间（§2.2）的契约。所有记录类型都是结构体数组。

### 9.1 记录格式

| 记录 | 存储的字段 | 说明 |
|---|---|---|
| `OrderRecords` | `col, idx, size, price, fees, side` | 每笔成交一行 |
| `LogRecords` | 37 个字段：执行前状态（`group, col, idx, cash, position, debt, free_cash, val_price, value`）、完整请求（`req_*`）、执行后状态（`new_*`）、结果（`res_size, res_price, res_fees, res_side, res_status, res_status_info`）、`order_id` | 每笔被考虑的订单一行 |
| `TradeRecords` | `col, size, entry_idx, entry_price, entry_fees, exit_idx, exit_price, exit_fees, pnl, ret, direction, status, parent_id` | 推导得出（§9.5） |
| `DrawdownRecords` | `col, peak_idx, start_idx, valley_idx, end_idx, peak_val, valley_val, end_val, status` | 由净值推导（§9.6） |
| `RangeRecords` | 每列的起点/终点/状态 | 来自 `barq-signals` 的区间 |

**id 是隐式的。** 记录的 id 就是它合并之后的位置，不单独存储（`OrderRecords::ids()`）。算上隐式 id，日志有 38 个字段，成交 14 个，回撤 10 个。

**顺序约定。** 记录按列分组、行号升序——不分组的运行天然就是列主序。共享组按执行顺序输出，由分析层重新排序。

### 9.2 合并每个线程的缓冲区

每个工作线程写入自己的缓冲区。缓冲区按分块顺序拼接（`OrderRecords::concat`、`Journal::concat`）。日志里的 `order_id` 是相对缓冲区的，合并时与 id 一样重新定基——全局计数器会让 id 取决于线程交错的顺序。因此不管用多少线程，合并结果都完全相同。

### 9.3 Sink

- `RecordSink::push_order(row, col, size, price, fees, side)`。
- `LogSink` 接收日志条目；对普通的 `OrderRecords` 它什么也不做。
- `SinkBuffer: RecordSink + LogSink + Send` 增加了 `with_capacity` 和 `merge`，由 `OrderRecords` 和 `Journal { orders, logs }` 实现。

写入经由 trait，这样阶段 2 可以复用记录层，而不必继承向量化层预分配的存储。每个线程的缓冲区按最坏情况预分配（每个元素一次成交，与参考实现的 `max_orders` 相同）；改用倍增扩容时，4 线程密集负载的吞吐下降了约 15%。

### 9.4 日志，以及公开日志发现的问题

`LogRecords::reasons()` 和 `reason_counts(col, n_cols)` 按列回答"为什么没成交"。两份日志很少会 `==`，因为 `res_*` 里的 NaN 让派生的 `PartialEq` 变成陷阱；要逐字段比较。日志第一次公开并与参考实现比对时，发现了其他测试都没发现的两个缺陷：默认 `min_size` 是 `0.0` 而不是 `1e-8`（一笔 `5e-9` 的订单在这里成交、在参考实现里被拒），以及串行路径里 `order_id` 没有跨组重新定基。

### 9.5 成交的三种视图

`crates/barq-records/src/trades.rs`，每个都接收 `(orders, close, n_rows, n_cols)`：

- `entry_trades`——每笔入场成交一条记录；离场按先进先出分配。
- `exit_trades`——每笔离场成交一条记录；入场取平均。
- `positions(trades, n_cols)`——每个持仓一条记录（按 `parent_id` 把成交分组）。

对同一批成交，它们会合理地给出不同的数量、盈亏拆分和胜率。未平仓的成交按最后一根收盘价计价。推导成交要求输入已排序，否则报错。

### 9.6 Mapped array 与回撤

- `ColMap::build(col_arr, n_cols)` 是稳定的计数排序。它能识别记录已经按列连续存放的常见情况，此时直接给出零复制的子切片；通用路径处理合并或过滤后的记录，结果完全相同。
- `MappedArray` 按列归约：`count`、`sum`、`mean`、`min`、`max`、`std(ddof)`、`idxmax`、`idxmin`、`reduce`、`top_n_mask`、`value_counts`、`stack_expand`（还原成 `Matrix`）。归约像参考实现一样跳过缺失值；空列取填充值。
- `find_drawdowns(ts, n_rows, n_cols)` 为每段回撤生成一条记录（峰值、谷值、结束点、`Active`/`Recovered`），并派生出各种持续时间、回撤幅度和恢复收益。
- **持续时间约定**（成交、回撤和信号区间都一样）：已结束的区间持续 `end - start`，未结束的持续 `end - start + 1`。两者用同一个约定会让平均值偏差最多一根 K 线。

---

## 10. 分析 barq-analysis

### 10.1 `Portfolio`

`barq_analysis::Portfolio<'a>`（`portfolio.rs`）只存订单记录、借用的收盘价（`FlexArray`）、形状、分组、每组初始现金、可选的基准，以及 `Scoring` 设置。其他一切都惰性推导，并在 `OnceLock` 里缓存一次，所以 `Portfolio` 是 `Sync` 的，每条序列最多计算一次。

构造方式：

- `from_records(orders, close, init_cash, n_rows, n_cols, scoring)`；
- `from_grouped_records(…, groups)`——分组共享时按 `(col, idx)` 重新排序记录；
- `from_orders(&FromOrders, threads)` 和 `from_signals(&FromSignals, threads)`——运行模拟（`threads > 1` 时并行）并包装结果。

`Scoring { ann_factor, risk_free, trades_type, incl_open }` 控制年化系数、无风险利率、指标使用哪种成交视图（§9.5）以及是否计入未平仓成交。

### 10.2 重建链

`series.rs` 只用记录重建各项余额：

```
orders → asset_flow → assets ─────────────┐
       → cash_flow  → cash（按组） ─────────┴→ value → returns
                                    close ─┘
```

`asset_value = assets × close`；每组 `value = cash + asset_value`；`returns` 是 `value` 的逐期收益（第一期相对于初始现金）。`gross_exposure` 和 `free_cash` 也以同样方式推导。

### 10.3 指标

按账户（组）计算、跳过 NaN，来自 `metrics.rs`：

- 收益类指标：`total_return`、`annualized_return`、`annualized_volatility`、`max_drawdown`、`sharpe_ratio`、`sortino_ratio`、`calmar_ratio`、`omega_ratio`、`tail_ratio`、`downside_risk`、`value_at_risk`、`cond_value_at_risk`（`DAILY = 365`，`VAR_CUTOFF = 0.05`）；
- 相对基准（`benchmark.rs`）：`information_ratio`、`beta`、`alpha`、`capture`、`up_capture`、`down_capture`；
- 以上每项的滚动版本（`rolling.rs`），基于 `barq_generic::rolling::rolling_apply_1d` 生成；
- `TradeStats { count, winning, losing, win_rate, profit_factor, expectancy, sqn, total_pnl, best, worst }`。

### 10.4 统计注册表

`stats.rs` 包含 28 个默认指标（`DEFAULT_METRICS`），每个都有键、标题、标签和计算函数：起止时间、区间、起止净值、总收益、基准收益、最大总敞口、已付手续费、最大回撤及其持续时间、成交计数、未平仓盈亏、胜率、最好/最差成交、平均盈利/亏损成交及其持续时间、盈亏比、期望值，以及 Sharpe、Calmar、Omega 和 Sortino 比率；`benchmark_metrics()` 另外加上 beta、alpha、信息比率和捕获率。`Select::{all, with_tags, keys, without_tags}` 按 `Tag`（Portfolio、Orders、Trades、Closed、Open、Winning、Losing、Drawdowns、Duration、Returns 等）过滤。标题和顺序与 vectorbt 的 `Portfolio.stats()` 一致，有测试夹具锁定每一行（§14.3）。

### 10.5 扫描结果汇总

- `traded_only(values)` 把从未交易的账户置为 NaN。**陷阱：**一个从不交易的列波动率为零，夏普比率可能是 `+inf`，于是朴素的 argmax 会选中它。`crates/barq/tests/sweep.rs` 锁定了这一点。
- `argmax` / `argmin` 跳过 NaN，但不跳过 `+inf`。
- `group_by(columns, level, values, Reduce::{Mean, Median, Max, Min, Count})` 按某个层级的每个标签归约（§3.1）；`best_column(columns, values)` 返回胜出的列及其标签。

---

## 11. 并行

### 11.1 所有并行的地方

| 位置 | 机制 | 工作单元 |
|---|---|---|
| `barq-sim` 的 `from_orders.rs`、`from_signals.rs`（`simulate_into_parallel`） | `std::thread::scope`，按连续的组分块 | 一个账户组（不分组时是一列） |
| `barq-expr` 的 `eval.rs:evaluate` | rayon `into_par_iter().for_each_init` | 一个标的的一列走完整个因子依赖图 |
| `barq-expr` 的 `polars_backend.rs`（feature） | Polars 自己的线程池 | Polars 惰性帧 |
| `barq-py` 的 `expr.rs`、`data.rs` | `py.detach`（释放 GIL） | 本身不是并行（§12.5） |

其他都不是并行的：记录、分析、技术指标、generic、信号和数据都在单线程上运行。`Portfolio` 是 `Sync` 的，但目前还没有任何地方对它做并行展开。

### 11.2 模拟器

- `n_threads` 被限制在 `[1, n_groups]`。组被切成每块 `ceil(n_groups / n_threads)` 个的连续分块；每个作用域线程拥有自己的 sink 和计数器。没有共享可变状态，没有串行段，也没有归约步骤。
- 结果按分块顺序合并并重新定基 id（§9.2）；如果多个分块出错，取序号最小的分块的错误。不管线程数多少输出都完全相同，测试断言串行和多线程运行的结果逐位一致。
- 并行单位是账户。互相独立的列每列一个单位；200 组参数 × 共享的 5 标的账户是 200 个单位；一个持有所有标的的组合只是一个单位，无法并行。
- 用作用域线程而不是工作窃取线程池：作用域线程强制参数是 `Sync` 的、状态是列本地的，而这才是关键性质。任务是整列或整组（10⁴–10⁶ 次迭代），调度开销可以忽略。如果将来负载均衡变得重要，可以在一个函数后面把它换成线程池。

实测（`cargo run --release --example scaling -p barq-sim`，20,000 根 K 线 × 256 列，4 核 Xeon，不分组；是在分组信号出现之前记录的）：

| 负载 | 1 线程 | 4 线程 | 加速比 | 效率 |
|---|---|---|---|---|
| 稀疏（每 20 根 K 线成交 1 次） | 3300 万元素/秒 | 1.21 亿元素/秒 | 3.62× | 90% |
| 密集（每根 K 线都成交） | 1500 万元素/秒 | 2500 万元素/秒 | 1.63× | 41% |

密集负载受内存带宽限制：6 个记录数组、每个 512 万个 `f64`，约 245 MB 的写入。纯计算的校准循环能扩展到 3.96×。（第一次校准测量是无效的——编译器把循环删掉了，"3.95×"是线程启动开销除以它自己。）

### 11.3 因子求值

用 rayon 按标的并行（§5.4）。每次求值一个并行区域；每个工作线程在它处理的各列之间保留暂存区；输出写入互不重叠的列切片。扩展能力受标的数量限制（16 个标的的面板最多只有 16 个任务），也受各标的工作量不均的影响（工作窃取以整列为粒度做平衡）。

### 11.4 为什么模拟循环用多线程而不是 SIMD

- **沿时间方向不可能：** 现金、持仓、负债和可用现金满足 `state[t] = f(state[t-1], bar[t])`。
- **跨列可以做但不划算：** 内核控制流高度分叉（七步数量换算链、买卖分派、手续费反解、粒度重新对齐、约 13 项不变量检查、止损跳空分支），有 14 种依赖数据的提前退出（即拒单原因），记录写入是不规则的分散写，而且 SIMD 本来就太窄（4–8 个通道，相比成千上万列）。

SIMD 适合的是因子和分析层（`diff`、`pct_change`、平移、滚动统计），顺序是：先做好布局，再依靠 LLVM 自动向量化，然后做性能剖析，最后才给两三个计算核心写 intrinsics（`std::simd` 还不稳定；稳定的选择是 `wide`/`pulp`）。

### 11.5 `Send` 约束

每个引擎实例都必须是 `Send` 的，并且不能有进程级全局状态。NautilusTrader 的 `Rc<RefCell<…>>` 和消息总线单例正是迫使它的 `BacktestNode::run()` 只能串行的原因。本工作区没有 `Rc`、没有 `RefCell`、没有全局变量（只有 `Portfolio` 内部的 `OnceLock` 缓存）；阶段 2 的组件应当使用 arena 加索引的所有权方式，任何消息路由都作为引擎的字段，而不是单例。

---

## 12. Python 层 barq-py

### 12.1 打包

`crates/barq-py` 构建一个包含两层的 wheel：Rust 扩展 `barq._barq`（PyO3 0.29、`abi3-py39`，由 maturin 构建）和 Python 包 `crates/barq-py/python/barq/`。可选依赖：`data`（pyarrow）和 `test`（pytest、vectorbt 1.1.0）。在仓库的 uv 环境里，`uv sync` 会构建它并以可编辑方式安装到 `.venv`（§16.2）。

### 12.2 数组不复制地传递——并且必须是 Fortran 序

`array.rs:columns_of` 把 NumPy 二维 `float64` 数组借用为列主序切片。它**拒绝 C 序数组**（除非某一维是 1），因为把行主序缓冲区当成列主序读取，会悄无声息地把时间和标的转置，得到一个看起来很正常的错误回测。非连续的视图也会被拒绝。Python 封装函数（`_matrix`、`_signals`、`_param`）在调用 Rust 之前用 `np.asfortranarray` 转换，测试断言被借用的数组永远不会被写入。

一维数组有歧义（每行一个值还是每列一个值？），所以每个调用点都声明它的含义（`OneDim::{PerRow, PerColumn}`）；模拟参数把一维数组视为每列一个值。

### 12.3 模拟入口

`barq.from_signals(close, entries, exits, *, open, high, low, init_cash, size, size_type, price, direction, fees, fixed_fees, slippage, min_size, max_size, size_granularity, lock_cash, allow_partial, raise_reject, log, accumulate, sl_stop, sl_trail, tp_stop, val_price, ffill_val_price, update_value, ann_factor=365, risk_free, trades="exit", incl_open, threads=1, cash_sharing=False, group_lens=None, call_seq="default", column_map=None)` 和 `barq.from_orders(close, *, size, size_type, …)` 与 Rust 结构体一一对应：

- 每个参数都可以是标量、一维的每列数组或完整的 `(T, C)` 数组；
- `cash_sharing=True` 且不给 `group_lens` 时，所有列成为一个账户；
- `call_seq` 接受 `"default"` 和 `"auto"`；
- `column_map` 把逻辑列映射到源价格列（§3.3）——价格保持 `[T, S]`，信号是 `[T, C]`；`Portfolio.price_nbytes` 报告保留的价格内存；
- 止损止盈需要 `high` 和 `low`。

### 12.4 其他绑定

- `barq.FactorSet.compile({...})`、`.evaluate({field: (T, S) 数组})` → `{name: (T, S) 数组}`、`names`、`required_fields`、`uses_future`；`barq.Expression`（单个因子）；`barq.alpha158()`、`barq.alpha360()`。只公开原生后端。
- `barq.crossed_above(a, b)`、`barq.factors`、`@barq.strategy`（§6.3）。
- `barq.data`（§4.5）。Python 解码器通过 Arrow C Data Interface 接收 `pyarrow.RecordBatch`（`arrow.rs`，唯一的 `unsafe` 代码，用 `#[allow(unsafe_code)]` 限定范围），只接受确切的 pyarrow 类型。
- `barq.InstrumentId`、`barq.BarType`。

### 12.5 `barq.Portfolio` 与 GIL

- Python 的 `Portfolio` 持有记录、按源宽度复制的一份收盘价、列映射、分组和评分设置。每个方法都会重建一个借用数据的 Rust `Portfolio` 并计算所请求的内容，所以 Rust 侧的缓存不会在 Python 调用之间保留；正因如此，`metrics()` 和 `stats()` 在一次调用里计算许多值。
- 方法：`stats(col=None, *, group=None)`、`metrics()`、`total_return`、`sharpe_ratio`、`max_drawdown`、`trade_count`、`traded_only`、`value/returns/cash(col, *, group)`、`assets`、`orders()`、`trades()`、`logs()`、`log_reasons()`、`reason_counts(col)`，以及 `n_rows`、`n_cols`、`n_groups`、`group_lens`、`n_fills`、`n_logs`、`price_nbytes`。记录以 NumPy 数组组成的 dict 返回。
- **GIL。** 因子求值、数据加载、`prepare` 和 `align` 会释放 GIL。`from_signals` / `from_orders` 不释放：它们的工作线程是并行运行的，但其他 Python 线程要等调用结束（§17.4）。
- 尚未公开：滚动指标、回撤记录、技术指标、barq-signals 工具、`CallSeq::Explicit`、Polars 后端。

---

## 13. 研究层 research/barq_lab

`research/` 是一个独立的 Python 包（`barq-lab`），用 barq 运行一套**预注册的因子研究协议**：由语言模型撰写假设和因子规格，由一套固定的协议决定哪些能留下来。模型永远看不到任何结果，因此无法针对数据做优化。健康的流水线绝大多数时候都在拒绝。

### 13.1 组成部分

- **账本**（`protocol/ledger.py`）：一个只追加的 `ledger.jsonl`，放在它自己的 git 仓库里（默认 `../barq-ledger`）。每条记录包含自身和前一条记录的 SHA-256 哈希；读取时校验整条链，每次追加都会提交。记录类型：`legacy`、`registered`、`run`、`control`、`oos_batch`、`forward`。账本统计样本内检验次数（N）和已用的样本外批次。
- **策略配置**（`policy.py`，按内容哈希做版本）：时间段（样本内 2021-01-01..2025-01-01，样本外 2025-01-01..2026-09-01）、数据（Binance 1 小时，BTC/ETH/SOL/BNB 对 USDT）、成本（手续费 0.001、滑点 0.0005、不模拟资金费率）、评估框架设置、规格限制、准入门槛、样本外预算（最多 3 批，间隔 90 天）以及前向跟踪规则。
- **流水线版本**（`versions.py`）：策略配置和判定代码的哈希。改提示词不会让以前的结果失效；改判定代码会把它们标记为过期。
- **卡片与规格**（`cards.py`、`types/factor_ts/spec.py`）：卡片陈述机制、交易对手、周期、预期表现、失效条件和固定的方向；日期、年份和具名事件会被拒绝，因为模型的训练数据覆盖了评估区间。规格是一个 barq 表达式网格，加上信号规则（`sign`、`zscore`、`band`）、方向和选择方式（`ensemble` 或预先声明的 `walk_forward`）。规格解析器与 `crates/barq-expr/src/parse.rs:validate` 保持一致（有测试检查两者一致）。限制条件约束网格大小、节点数、自由参数比例以及与已尝试结构的相似度。

### 13.2 工作流程

1. `propose`——模型把卡片和规格写入 `pending/`（它只能看到已尝试过的标题，永远看不到结果）；格式问题最多退回三次。
2. `review` / `show`——人工阅读和编辑。
3. `approve`——规则检查，再加上在合成 K 线上的**契约检查**：规格能编译、不被 NaN 主导、不是常数，并且**前缀一致**（追加 K 线永远不会改变之前的值——没有前视）；然后做一次冒烟回测。通过的规格登记到账本。
4. `run`——每个已登记的规格在样本内只跑一次。失败的运行也计入 N，因为数据已经被读过了。
5. `oos`——在批次预算和间隔限制之内，把当前样本内通过的规格分批做样本外检验。
6. `forward`——幸存者在比登记时间和样本外区间都更新的 K 线上做影子跟踪；日收益的 z 分数跌破 −2 就退役。
7. `report`、`calibrate`（阴性对照——随机因子、打乱时间的因子，以及一个必须被拒绝的故意前视因子；阳性对照只做记录）、`import-legacy`（之前的试验也计入 N）。

### 13.3 评估框架及其对 barq 的使用

`types/factor_ts/harness.py` 用 `barq.FactorSet.compile(...)` 编译规格，拒绝 `uses_future`，在 Fortran 序的字段上求值，把成员因子转成量化后的目标权重（在 `t` 决定、在 `t + 1` 执行，只在变化时下单），然后运行**一次** `barq.from_orders(close, targets, size_type="targetpercent", cash_sharing=True, group_lens=…, call_seq="auto", column_map=…, ann_factor=8760, threads=4)`，把每个成员 × 成本倍数（0×、1×、2× 手续费）作为独立的共享账户。准入门槛要求：扣费后的 t 统计量超过随 N 上升的 Bonferroni 阈值（`Φ⁻¹(1 − α / 2N)`）、t 统计量不能大得可疑（≤ 8）、两倍成本下夏普为正，并且在各成员和各资产之间一致。最接近的 Alpha158 因子（`barq.alpha158()`）只作参考报告。

---

## 14. 验证

### 14.1 与已提交测试夹具做差分测试

每一层的验收标准都是：相同输入下与参考实现结果一致。

1. `tests/parity/generate_*.py` 里的 Python 生成脚本运行参考实现，把测试夹具写入 `tests/fixtures/`。
2. 测试夹具提交进仓库。Rust 测试回放它们，所以 `cargo test` 既不需要 Python 也不需要参考实现。
3. 输入是手工挑选的边界情况加上固定种子（`20260918`）的随机情况，而不是基于性质的测试：重新生成的测试夹具出现差异，这个差异本身就是一个发现。
4. 记录数组逐字段比较，从不通过聚合值比较。浮点按参考实现的容差比较（`agree`：同为 NaN、同号无穷，或 `is_close`）；整数（`col`、`idx`、买卖方向、状态、id）必须完全相等。

测试夹具格式：带表头行的 TSV，开头可以有记录生成脚本、参考版本和种子的 `#` 注释行；数组用 `;` 分隔，多个输出用 `|` 分隔；广播前缀 `s:` 标量、`c:` 每列、`r:` 每行、`f:` 完整的列主序矩阵。`shared_accounts.json` 存放完整的用例。

### 14.2 粒度：一次调用、一串调用、整条流水线

- `crates/barq-exec/tests/parity.rs`——一个测试夹具就是一次内核调用，所以出错时能指出是换算链的哪一步坏了。
- `crates/barq-exec/tests/parity_sequence.rs`——把调用串起来，把内核自己的输出往后传（只有初始现金来自测试夹具）；能捕捉单次调用发现不了的状态传递问题和残差。
- `crates/barq-analysis/tests/parity_analysis.rs`——从订单经净值、收益一直到十几个标量指标；最后才建，因为它无法定位出错的位置。

### 14.3 各层的一致性测试

| 测试 | 测试夹具（行数） | 参考 | 容差 |
|---|---|---|---|
| `barq-exec/tests/parity.rs` | `execute_order.tsv`（474） | vectorbt `execute_order_nb` | `agree`（相对 1e-9 / 绝对 1e-12） |
| `barq-exec/tests/parity_sequence.rs` | `order_sequence.tsv`（1,504 步） | 串联的 `execute_order_nb` | `agree` |
| `barq-sim/tests/parity_from_orders.rs` | `from_orders.tsv`（290） | `simulate_from_orders_nb` | `agree`，记录逐字段 |
| `barq-sim/tests/parity_from_signals.rs` | `from_signals.tsv`（181） | from_signals 循环与止损引擎 | `agree` |
| `barq-sim/tests/parity_signals.rs` | `signals.tsv`（5,088，穷举） | 信号处理各阶段 | `is_close` |
| `barq-sim/tests/parity_cash_sharing.rs` | `cash_sharing.tsv`（48） | 分组的 from_orders，默认顺序与自动顺序 | 相对 1e-9 |
| `barq-records/tests/parity_trades.rs` | `trades.tsv`（267） | Entry/Exit trades、positions | `is_close`，整数精确 |
| `barq-records/tests/parity_drawdowns.rs` | `drawdowns.tsv`（998） | 回撤 | 相对 1e-12 |
| `barq-records/tests/parity_mapped.rs` | `mapped.tsv`（28,367） | MappedArray 归约 | 相对 1e-12 |
| `barq-generic/tests/parity_generic.rs` | `generic.tsv`（1,650） | `vectorbt.generic.nb` | `is_close` |
| `barq-indicators/tests/parity_indicators.rs` | `indicators.tsv`（480） | `vectorbt.indicators.nb` | `is_close`；扫描结果与单独运行逐位相同 |
| `barq-signals/tests/parity_signals_gen.rs` | `signals_gen.tsv`（5,782，穷举） | `vectorbt.signals.nb` | 精确 / 1e-12 |
| `barq-analysis/tests/parity_analysis.rs` | `analysis.tsv`（37） | 端到端 | `is_close` |
| `barq-analysis/tests/parity_metrics.rs` | `metrics.tsv`（2,047） | `vectorbt.returns.nb` | 相对 1e-9 |
| `barq-analysis/tests/parity_stats.rs` | `stats.tsv`（72） | `Portfolio.stats()`，全部 28 行 | 相对 1e-9；标题和顺序精确 |
| `barq-expr/tests/parity_qlib.rs` | `qlib_alpha158.tsv`（240）、`qlib_alpha360.tsv`（64） | Qlib `Alpha158DL`/`Alpha360DL` | 相对 1e-6（`CORR*`/`CORD*` 为 1e-4）；原生与 Polars 两个后端 |

结构性测试补充了一致性测试：`crates/barq/tests/sweep.rs`（一次扫描就是一次运行；多线程与串行逐位相同；标签能追溯到参数；`+inf` 夏普陷阱）、`crates/barq/tests/factor_sweep.rs`、`crates/barq/tests/factor_path.rs`、`crates/barq-analysis/tests/shared_accounts.rs`（手算的共享账户），以及 §5.5 中 barq-expr 的单元测试。

### 14.4 变异测试

每一层都通过手工施加变异、统计有多少被测试捕获来检验；每个幸存的变异都要论证为不可达或等价。完整矩阵在 `tests/parity/README.md` 里（例如 generic 捕获 24/25，indicators 24/24，analysis 43/47）。

内核的经验可以推广：五个内核变异里捕获了四个；幸存的那个把 `is_close_or_less` 换成了 `<=`，两者只在 `1e-9` 以下有差别。解决办法是把测试夹具瞄准边界，并找到一个离散的放大器——`size_granularity` 把看不见的差别变成整整一个单位（§7.5）。归零有两个冗余机制（入口处的归零和 `add`）；按设计，单独去掉任何一个都检测不出来，两个都去掉则只有串联测试能发现（现金变负，让拒单变成错误）。

### 14.5 Python 测试

`crates/barq-py/tests/`：

- `test_bindings.py`——在同一个解释器里运行 vectorbt 的差分测试（13 个信号用例、14 个订单用例、4 种止损配置、10 个日志用例、所有统计行；整数精确，浮点 1e-12，统计 1e-9），以及结构性测试（线程确定性、拒绝 C 序、形状和枚举错误、广播、未交易掩码、记录 dtype、不通过借用数组写入、日志和 `reason_counts`）。
- `test_shared_accounts.py`——已提交的 `shared_accounts.json` 基准（48 个用例，也用 3 个线程重跑）和一个实时调用 vectorbt 的检查。
- `test_column_map.py`、`test_data.py`、`test_examples.py`、`test_expressions.py`、`test_strategy.py`。

需要 vectorbt 的测试在未安装时会跳过；仓库的 uv 环境（Python 3.14）不包含 vectorbt，所以要在执行过 `pip install -e 'crates/barq-py[test]'` 的环境里运行它们。

### 14.6 重新生成测试夹具

```bash
python -m venv .venv-parity
.venv-parity/bin/pip install "vectorbt==1.1.0" "plotly<6"
for g in tests/parity/generate_*.py; do .venv-parity/bin/python "$g"; done
cargo test --workspace
```

Qlib 生成脚本在 Qlib 环境里运行（`conda run -n rdagent4qlib python tests/parity/generate_qlib_alpha158.py`），只把 Qlib 最底层的特征提供者替换成一个确定性的面板（不规则日期、注入的 NaN），所以不会下载任何行情数据。`generate_shared_accounts.py` 使用一个可编辑安装的 vectorbt 检出。重新生成之后要阅读 diff：任何变动的行都是一个发现。完整流程见 `tests/parity/README.md`。

---

## 15. 基准测试与性能

### 15.1 模拟循环对比 vectorbt

`benchmarks/` 把相同的负载分别跑在 barq 和 vectorbt 的两个引擎上：

```bash
cargo run --release --example bench -p barq-sim      # → benchmarks/results/barq.tsv
.venv-parity/bin/python benchmarks/bench_vectorbt.py  # → benchmarks/results/vectorbt.tsv
python benchmarks/compare.py                          # 正面对比
python benchmarks/compare.py --check-baseline         # 回归门禁
```

- `workloads.tsv` 定义了十个 `from_orders` 负载，在成交率（密集受内存带宽限制，稀疏受计算限制）、长宽比和每根 K 线的工作量上各不相同。
- **工作量不同就不比较。** 两边的运行程序用同一个公式生成输入，这个公式各自实现了一遍；两边都报告数组的校验和与成交数，不一致时 `compare.py` 直接中止。
- **内核对内核。** vectorbt 的调度封装在进入循环前要校验和准备约十八个数组，耗时约为 2 倍；有一次通过封装比较，得出的是 5.3× 而不是 2.7×。主要对比层级直接计时 vectorbt 的 `simulate_from_orders_nb`。
- 在 1 个、2 个和全部线程下各取 7 次中最好的一次；运行时断言线程数不改变结果。
- **回归门禁：** 只对单线程行设 10% 阈值（单线程噪声中位数 1.3%、最差 3.7%；4 线程噪声高达 47%）；如果所有负载同步偏移，会标记为可能是机器负载。

在 2.10 GHz 的 4 核 Xeon 上，内核层级，三个时段的结果：

| 对比 | 中位数范围 |
|---|---|
| barq 对 vectorbt Numba | 2.6×–3.3× |
| barq 对 vectorbt Rust | 3.5×–4.9× |
| barq 4 线程对 vectorbt Numba | 9.7×–12.1× |
| vectorbt Rust 对 vectorbt Numba | 0.74×–0.82× |

之所以给出范围，是因为这个比值在不同时段之间变动了 28%：机器繁忙时 Numba 循环比 barq 损失更多。应当引用这个形态，而不是某一个数。

### 15.2 其他性能工具

- `cargo run --release --example scaling -p barq-sim`——线程扩展性（§11.2）。
- `cargo run --release --example scale -p barq-expr -- <rows> <cols>`——在真实规模的面板上逐个算子和整个 Alpha158 的实际耗时；O(window) 的计算核心或无上限的缓存会在这里暴露出来。
- `cargo bench -p barq-expr --features polars`——用 criterion 在 1024 × 16 上比较原生和 Polars 后端（§5.6）。

### 15.3 因子引擎的演进

因子引擎在本分支上分五步重做；每一步都在同一时段交替运行新旧构建测量（Alpha158，4 线程）：

| 改动 | 效果 |
|---|---|
| O(1) 滚动计算核心、按列并行的算子、释放 GIL | 2520×200：7.87 s → 0.96 s |
| 中间结果在最后一次使用后释放、字面量广播、字段借用 | 2520×500 内存峰值：3.5 GB → 1.7 GB |
| O(log n) 顺序统计量、单调队列、基于幂和的偏度/峰度 | 2520×200：860 → 629 ms |
| 依赖图、按标的并行求值 | 2520×200 约 −20%；2520×16：160 → 75 ms；内存 1695 → 1591 MB |

---

## 16. 开发流程

### 16.1 工具链与配置

- `mise.toml`：Rust 1.96.0、Python 3.14.6、uv 0.11.33；自动创建 `.venv`；`mise run sync` 执行 `uv sync`。
- 根目录 `Cargo.toml`：edition 2024、`rust-version = 1.85`、Apache-2.0。Lint：`unsafe_code = "deny"`、`missing_debug_implementations = "warn"`、`clippy::float_cmp = "deny"`、`clippy::too_many_arguments = "allow"`（忠实移植参考实现的循环需要）。Release profile：`lto = true`、`codegen-units = 1`、`opt-level = 3`（撮合内核的 debug 构建慢约 50 倍，计时没有意义）。
- Features：`barq-expr/polars`；`barq/data`、`barq/polars`。
- `rustfmt.toml`：`max_width = 100`。根目录 `pyproject.toml` 是一个 uv 工作区（`barq-dev`），依赖可编辑安装的 `barq[data]`，并有一个 `research` 依赖组；ruff 和 pyright 针对 `examples`、`research` 和 Python 包做了配置。

### 16.2 常用命令

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo test -p barq-expr --features polars          # 包含 Polars 一致性测试

uv sync                                            # 构建并把 barq 安装到 .venv
uv run --with pytest python -m pytest crates/barq-py/tests -q
uv sync --reinstall-package barq                   # 改了 Rust 代码之后
```

仓库里没有 CI 配置；以上命令都是手动运行的。

### 16.3 示例

| 示例 | 展示内容 |
|---|---|
| `examples/sweep.py` | `@barq.strategy` 均线交叉，7 组参数 × 4 个合成标的，28 列，4 线程 |
| `examples/portfolio.py` | 用 `from_orders(targetpercent, cash_sharing, call_seq="auto")` 做共享现金再平衡；用 `group_lens` 让每组参数一个共享账户 |
| `examples/multi_data.py` | 从 catalog/Parquet/CSV 读取 K 线 + 资金费率 + 新闻，as-of 和窗口对齐，用 `column_map` 扫描而不复制价格 |
| `examples/crypto.py` | 用自定义解码器在缓存的 Binance K 线上做均线扫描（`--demo` 使用合成数据） |
| `examples/fetch.py`、`binance.py`、`download_binance_vision.py` | 仅限示例的下载器，写 CSV 缓存或 Parquet catalog |
| `crates/barq/examples/factor_sweep.rs` | Rust：因子 → `SignalSweep` → `Portfolio::from_signals` |

---

## 17. 路线图与已知缺口

### 17.1 阶段 2：带 NautilusTrader 兼容 API 的事件驱动层

这部分目前没有任何实现，是设计目标。

- **兼容指源码层面可移植：** 改一下 import，保持方法名和签名不变，不做链接（附录 A）。
- **以 NautilusTrader v2（Rust 重写版）为目标**，而不是 v1。值得注意的改名：`on_quote_tick` → `on_quote`、`on_trade_tick` → `on_trade`、`on_order_book*` → `on_book*`；`cache.quote_tick(s)` → `cache.quote(s)`；`on_event` 拆分成 `on_time_event` / `on_order_event` / `on_position_event`；`self.msgbus` 被 `publish_message` / `subscribe_topic` / `unsubscribe_topic` 取代；`modify_order` 接收 `ClientOrderId`；`cancel_all_orders` 默认只作用于本策略；`is_running` 和 `state` 变成方法；`BarType` 是枚举。
- **形态：** 一个持有 `core: StrategyCore` 和用户状态的结构体，用 `barq_strategy!` 宏注册，实现 `DataActor`，在 `on_start` 里订阅 K 线，在 `on_bar` 里 `submit_order`。
- **最小接口：** `Strategy::new(config)`、`self.config`、`cache.instrument()`、`order_factory.market()`、`submit_order`、`close_all_positions`、`portfolio.is_net_flat/long/short`、`subscribe_bars`、`register_indicator_for_bars`、`indicators_initialized`、`on_start`/`on_bar`/`on_stop`。
- **并行扫描 API：** `ParamGrid::new().sweep(…).per_instrument([...])`——`per_instrument` 把标的放在并行轴上；`instruments` 形式则让一个策略同时看所有标的（配对、横截面），只在参数之间并行。`Portfolio::from_strategy::<S>(&data, grid).run()` 返回与 `from_signals` 相同的 `Portfolio`。
- **K 线撮合，**如果跟随 NautilusTrader：L1 订单簿；每根 K 线变成四个 tick O → H → L → C，最高价和最低价中离开盘价更近的那个排在前面；成交量分摊到各段；以订阅的最细时间粒度为准。
- **阶段 1 已经遵守的约束：** 内核可以针对单根 K 线独立运行（§7）；记录经由 `RecordSink` 写入（§9.3）；没有 `Rc`/全局变量（§11.5）。持仓查找必须有索引，绝不能线性扫描（NautilusTrader v2 在累积了数千次成交的净额持仓上比 v1 慢 1.64 倍，原因正是一次线性扫描）。

### 17.2 因子引擎

- **分批输出**——按因子分批求值，写入 sink 或 Arrow/Parquet，让内存峰值受批次大小而不是整个因子集约束（§5.7）。
- **分阶段的横截面算子**——`CSRank`、`CSZScore`、中性化、缩尾；在这些节点处切分依赖图（§5.7）。
- **更多逐元素算子**——`Exp`、`Sqrt`、`Clip`、`FillNa`、`IsNan`、掩码。
- **单个标的内部的并行**，用于窄面板（标的数少于核数）：可以把依赖图中互不依赖的节点分给多个线程。
- **Polars 后端**——只有在下游 Polars 流水线需要时才值得优化（§5.6）。

### 17.3 待评估的想法

尚未排期；在函数式策略（§6.3）已经存在的前提下再做评估：

- 带目标函数和独立验证区间的自动参数搜索；
- 跨运行的因子缓存（数据标识、失效、内存上限——先测量单次扫描内的复用程度）；
- 按名字引用因子、复用表达式语言的声明式交易规则。

### 17.4 已知问题

编写本文时发现；都不影响结果的正确性。

- `crates/barq-sim/examples/bench.rs` 把实现名写成 `vbt`，而 `benchmarks/compare.py` 和 `baseline.tsv` 期望的是 `barq`，所以回归门禁目前匹配不到 barq 的行。
- `benchmarks/README.md` 和 `bench_vectorbt.py` 仍然写着 barq 没有 Python 层。
- `reject_prob` 在内核中已实现，但两个循环都没有提供随机数（§7.6）。
- Rust 中有 `CallSeq::Explicit`，但 Python 只接受 `"default"` 和 `"auto"`。
- Python 的 `from_signals` / `from_orders` 在整个模拟期间持有 GIL（§12.5）。
- Python 的 `Portfolio` 每次调用都重建 Rust `Portfolio`，所以缓存不会在调用之间保留（§12.5）。
- `Symbol`/`Venue` 是没有驻留机制的 `Arc<str>`，尽管某处模块注释描述的是驻留的句柄（§3.6）。
- `from_orders` 和 `from_signals` 在一些实现细节上不同：串行路径的预分配（`n_rows × n_cols` 对比按需增长）和工作线程 panic 的处理（映射为 `Error::Invariant` 对比直接传播）。
- 有些文档注释已经过时：`barq-analysis` 的 crate 文档说共享现金尚未实现（其实已实现，§10.1）；`from_orders.rs` 用"crates.io 镜像不可用"来解释为什么用作用域线程（rayon 现在已经是依赖，§11.2）。
- `research/barq_lab/propose.py` 提到 `tests/test_propose.py`；它描述的检查实际在 `research/tests/test_pipeline.py::test_proposals_never_see_results`。
- 两个 Qlib 测试夹具是用不同版本的 Qlib/NumPy/pandas 生成的（记录在各自的表头里）。

---

## 附录 A 许可与代码来源

barq 采用 Apache-2.0 许可（`LICENSE`，工作区 `license = "Apache-2.0"`）。两个参考项目只作为规格说明和测试基准使用；**没有从任何一个复制代码**，每一行都是依据公开语义独立编写的实现。

- **NautilusTrader 采用 LGPL-3.0-only。** 链接它会把 LGPL 义务带进本项目，而 Rust 是静态链接的，许可证第 4 条会要求提供可重新链接的目标文件。barq 不链接它。API 名称和方法签名不受保护，所以用同样的方法名做一套净室实现的 API 是允许的。
- **vectorbt 采用 Apache-2.0 附加 Commons Clause**，禁止售卖该软件（包括价值来源于它的托管或咨询服务）。barq 没有复制它的任何代码；重新实现其文档化的行为是允许的。
- **插件路线走不通。** `nautilus-plugin` 只是一个客户端 SDK（一个 manifest、`nautilus_plugin_init`、零大小的宿主函数表，其测试断言 `size_of::<HostVTable>() == 0`），live 节点会以 "requires host-side plug-in support; nautilus-plugin is the guest SDK only" 拒绝。它的组件契约是把策略送*进* NautilusTrader，不能替换引擎。
- **规则。** 可以读，不能抄。测试夹具可以以数据形式（输入和期望输出）从上游测试派生，但绝不能直接拷入上游的测试文件。文档只引用路径和简短的引文。
- 参考实现只在外部环境里运行（`.venv-parity`、一个 Qlib 的 conda 环境、一个可编辑安装的 vectorbt 检出），用于生成提交进仓库的测试夹具。它们都不是 crate 或 wheel 的依赖；vectorbt 只出现在可选的 `test` 依赖里。

## 附录 B 术语表

| 术语 | 含义 |
|---|---|
| 列（column） | 一条被模拟的序列：一个标的，或一个（参数组合，标的）对（§3.1） |
| 账户 / 组（account / group） | 共享现金的若干列；不分组表示每列一个账户（§8.3） |
| 调用顺序（call sequence） | 共享账户在一根 K 线内执行各列的顺序 |
| 成交（fill） | 一笔已执行的订单；`OrderRecords` 的一行 |
| 日志行（log row） | 一笔被考虑的订单，无论是否成交，含执行前后的状态（§9.1） |
| 拒单（rejection） | 无法成交的订单，以状态而不是错误的形式返回（§3.5） |
| 因子（factor） | 一个命名表达式，产出一个 时间 × 标的 的矩阵（§5） |
| 节点 / 槽位（node / slot） | 因子依赖图中去重后的一个操作 / 它可复用的暂存序列（§5.3） |
| `column_map` | 逻辑列 → 源价格列，使扫描不复制价格（§3.3） |
| 阶段 1 / 阶段 2 | 向量化层（已存在）/ 事件驱动层（计划中） |

## 附录 C 文件索引

| 主题 | 文件 | 章节 |
|---|---|---|
| 矩阵、广播、标签、容差、错误 | `crates/barq-core/src/{matrix,flex,columns,math,error}.rs` | §3 |
| 枚举、标识符、K 线、定点数 | `crates/barq-model/src/{enums,ids,bar,types}.rs` | §3.6 |
| 数据加载与对齐 | `crates/barq-data/src/lib.rs`、`crates/barq-py/python/barq/data.py` | §4 |
| 表达式 | `crates/barq-expr/src/{parse,graph,eval,window,order,polars_backend,alpha158,alpha360}.rs` | §5 |
| 信号、扫描、技术指标 | `crates/barq/src/sweep.rs`、`crates/barq-signals/src`、`crates/barq-indicators/src`、`crates/barq-py/python/barq/strategy.py` | §6 |
| 撮合内核 | `crates/barq-exec/src/{types,kernel}.rs` | §7 |
| 模拟 | `crates/barq-sim/src/{from_orders,from_signals,signals,groups}.rs` | §8 |
| 记录 | `crates/barq-records/src/{lib,logs,trades,mapped,drawdowns}.rs` | §9 |
| 分析 | `crates/barq-analysis/src/{portfolio,series,metrics,benchmark,rolling,stats,trade_stats,aggregate}.rs` | §10 |
| Python 绑定 | `crates/barq-py/src/{lib,array,params,parse,expr,data,arrow}.rs`、`crates/barq-py/python/barq/` | §12 |
| 研究协议 | `research/barq_lab/` | §13 |
| 一致性测试框架 | `tests/parity/`、`tests/fixtures/`、`crates/*/tests/` | §14 |
| 基准测试 | `benchmarks/`、`crates/barq-sim/examples/{bench,scaling}.rs`、`crates/barq-expr/{examples/scale.rs,benches/backends.rs}` | §15 |
