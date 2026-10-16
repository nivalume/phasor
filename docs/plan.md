# phasor 功能计划：对齐 VectorBT PRO

目标：实现 VectorBT PRO（下称 PRO）公开文档列出的全部功能，并补上 PRO 没有的保证金账户、强平和持仓费。
功能清单来自 2026-10 对 [vectorbt.pro/features](https://vectorbt.pro/features/overview/) 的整理；PRO 没写参数名的功能只描述行为。

状态以代码为准。最后核对：2026-10-16，分支 `claude/dreamy-dijkstra-39adnh`。

图例：✅ 完成　🟡 部分完成　⬜ 未开始　➕ PRO 没有、phasor 有

---

## 1. 功能总表

| # | 功能 | 状态 | 已有 | 还缺 |
|---|---|:-:|---|---|
| 1 | [模拟入口与回调](#21-模拟入口与回调) | 🟡 | `from_orders`、`from_signals`（含 `order_mode`）、`from_order_func`（flex、`row_wise`）、两个循环的回调钩子、Python 回调（直接调用）；内核、止损判定、挂单判定、现金流入账、记录写入已公开 | 常见回调的表达式 DSL |
| 2 | [订单与仓位](#22-订单与仓位) | ✅ | 全部仓位类型、下单约束、价格选项（含 `nextvalidopen`、按根的 `from_ago`）、买卖价、`call_seq`（auto 按现金影响、显式顺序） | — |
| 3 | [限价单与挂单](#23-限价单与挂单) | 🟡 | 限价单、突破单、有效期（按行数或按时间）、挂单冲突 | 语义评审 |
| 4 | [止损](#24-止损) | ✅ | 止损、止盈、移动止损、时间止损（按行数或按时间）、四种阶梯、`delta_format`、止损限价单、不经模拟的止损出场、回调里的动态阶梯 | — |
| 5 | [账户与资金](#25-账户与资金) | ✅ | 共享现金、入金出金、收支、分红、杠杆、合约乘数、初始持仓、`init_cash="auto"`、负价格、连续合约、汇率换算 | — |
| 6 | [保证金、强平、持仓费](#26-保证金强平持仓费) ➕ | ✅ | 全仓和逐仓保证金、强平（含部分强平、分档、ADL、融资融券）、资金费、隔夜融资、借贷利息、标记价格；和 NautilusTrader 对照一致 | — |
| 7 | [多周期与交易时段](#27-多周期与交易时段) | ✅ | 粗周期对齐到细周期、聚合回粗周期、交易时段、强制平仓 | `Portfolio.resample`（归在组合分析） |
| 8 | [状态延续与增量回测](#28-状态延续与增量回测) | ⬜ | — | 状态快照、`pf.update`、拼接、`save`/`load`、stepper |
| 9 | [数据](#29-数据) | 🟡 | 本地 CSV、Parquet、目录读取，按可用时间对齐，合成数据 | 数据源连接器、数据库存储、增量更新 |
| 10 | [指标](#210-指标) | 🟡 | 8 个 Rust 指标和参数扫描，因子 DSL（Alpha158、Alpha360） | Python 接口、指标工厂、第三方库、WorldQuant 101、流式 |
| 11 | [信号与标签](#211-信号与标签) | 🟡 | 带种子随机信号，信号区间，4 种 ML 标签 | Python 接口、日历信号、统计表 |
| 12 | [组合分析](#212-组合分析) | 🟡 | 记录、收益与风险指标、回撤、敞口、MAE/MFE、事件研究 | 切片拼接重采样、图表、形态搜索、部分 Python 接口 |
| 13 | [参数优化与验证](#213-参数优化与验证) | 🟡 | 策略参数扫描，交叉验证切分，PSR、deflated Sharpe、PBO | 通用参数化、日历切分、置换检验、组合优化 |
| 14 | [性能与分布式](#214-性能与分布式) | 🟡 | 账户组多线程（逐位一致），指令数回归检查 | 分块、断点续跑、多进程、Ray、缓存 |
| 15 | [工具链](#215-工具链) | ⬜ | — | 配置文件、CLI、定时任务、通知 |
| 16 | [AI 接口](#216-ai-接口) | ⬜ | — | 文档检索、MCP 服务、agent skills |

---

## 2. 各模块

### 2.1 模拟入口与回调

回调不是事件驱动：它在现有的逐根 K 线循环里给用户代码留位置，用同一个撮合内核，输出同样的记录。README 提到的事件驱动层（NautilusTrader 风格）不是 PRO 的功能，不在本计划内。

- [x] `from_orders`：逐笔数量、价格、方向；共享现金和分组（`group_lens`）；串行和多线程逐位一致
- [x] `from_signals`：入场出场信号、方向、冲突规则、加仓
- [x] `order_mode=True`：`from_signals` 里 `size` 就是订单，止损、限价单、价格选项照常可用
- [x] 自定义循环可用的零件：撮合内核（`execute_order`、`execute_validated`）、止损判定、记录写入（`RecordSink`、`LogSink`）、`Portfolio::from_records`、`phasor_core::random::uniform`
- [x] 公开现金流入账函数：`phasor_sim::cash`（`book` 记任意一笔现金流并写记录，`deposit`、`pay_dividends`、`earn`）
- [x] 公开挂单判定：`phasor_sim::limit`（`limit_price`、`last_row`、`fill_price`）
- [x] Rust 回调 trait（`phasor_sim::callbacks`）：`OrderFunc` 的 `pre_group`、`pre_segment`、`order`、`flex_order`、`post_order`、`post_segment`；只读的 `Context`（现金、净值、各列持仓、债务、估值价、收盘价，价格不含合约乘数）；每组一个 `State`，随记录按组返回（即 `in_outputs`）。`NoCallbacks` 把全部调用在编译期去掉；按账户组照常多线程。`pre_sim` 不需要：回调对象在运行前构造
- [x] `from_order_func`（`simulate_from_order_func`）：`FLEX` 时一根 K 线任意多笔、任意列和顺序；`Flow::SkipSegment`、`Flow::StopGroup`（停止下单，剩下的 K 线照常记现金流、持仓费和强平）；`Schedule::RowWise` 每根 K 线先跑完所有组再进下一根，单线程，记录和按组跑相同
- [x] 回调不能绕过记录：改成杠杆或保证金账户、改列的账户类型都报错；价格的 `±inf` 和数组里同义
- [x] `from_signals` 的回调（`simulate_from_signal_func`）：`adjust`（测试前改止损）、`signals`（四个信号和 `size`、`size_type`，照常走冲突、累加、止损、限价单、`order_mode`）、`post_order`、`post_segment`；和 `from_ago` 一起用报错
- [x] 按风险算仓位：`risk_size(value, risk, entry, stop, multiplier)`，在 `order` 或 `signals` 回调里用（测试：1% 风险、保本止损）
- [x] Python 回调，直接调用（每次调用拿 GIL，作调试和试规则用）：`phasor.from_order_func(close, order_func, flexible=, row_wise=, pre_group_func=, pre_segment_func=, post_order_func=, post_segment_func=)`，`from_signals(signal_func=, adjust_func=, post_order_func=, post_segment_func=, pre_group_func=)`；回调抛出的异常原样抛给调用方；组状态在 `pf.in_outputs`
- [ ] 常见回调的表达式 DSL（Python 写、Rust 跑），见 [§4](#4-待决定)

### 2.2 订单与仓位

- [x] 仓位类型：`Amount`、`Value`、`Percent`、`TargetAmount`、`TargetValue`、`TargetPercent`、`ValuePercent`、`Percent100`、`ValuePercent100`、`TargetPercent100`，两个入口都接受
- [x] 下单约束：`min_size`、`max_size`、`size_granularity`、`allow_partial`、`raise_reject`、`log`
- [x] 单笔买单资金上限 `cash_limit`
- [x] 随机拒单 `reject_prob`：随机数由 `(seed, row, col)` 算出，和线程数、分组无关
- [x] `call_seq="auto"`
- [x] 成交价：`price_type`（`Price`/`Open`/`Close`），`price="nextopen"`/`"nextclose"`，`from_ago`
- [x] 买卖价分离 `bid_price`、`ask_price`
- [x] 超出 K 线范围的价格 `price_area_vio_mode`（`ignore`/`cap`/`error`）
- [x] 自定义调用顺序：Python 的 `call_seq` 可以是 `(n_bars, n_cols)` 整数数组，每行每组给出组内列的执行顺序；同一行重复的下标报错
- [x] 带杠杆或保证金列的运行里，`call_seq="auto"` 按订单对可用现金的影响排序（`approx_cash_need`）：减仓部分释放，开仓部分占用（现金列按 `1/leverage`，保证金列按初始保证金）。持仓费在所有订单之后的收盘收取，不影响顺序；其他运行仍按参考实现的订单价值排序
- [x] `from_ago` 按列或按行设置：在执行那根读取，Python 里可以是整数、每根一个的一维数组或和 `close` 同形状
- [x] 跳过缺失值的价格选项 `price="nextvalidopen"`/`"nextvalidclose"`：在下一根有价格的 K 线执行上一根有价格的 K 线决定的订单（Python 里换算成按根的 `from_ago`）

### 2.3 限价单与挂单

目前只在 `from_signals`。每列最多一张挂单，状态在 `ColumnState.limit`。

- [x] `order_type`（`market`/`limit`，可以每列一个）
- [x] `limit_delta` 和 `delta_format`（`percent`/`percent100`/`absolute`/`target`）；`limit_reverse` 即突破单
- [x] `limit_order_price`（`limit`/`hardlimit`/`close`）、`limit_delay`；限价单不加滑点
- [x] 有效期 `limit_tif`、`limit_expiry`，按行数
- [x] 挂单冲突：`upon_adj_limit_conflict`、`upon_opp_limit_conflict`、`upon_adj_stop_conflict`、`upon_opp_stop_conflict`
- [x] 按时间给有效期：`time_delta_format="index"` 时 `limit_tif` 是时长、`limit_expiry` 是时刻、周期末或每天的某个时刻，按 `index` 换算成行数（`phasor/_timing.py`）
- [x] `from_orders` 式的限价单：PRO 的 `from_orders` 也没有限价单，用 `from_signals(order_mode=True)`，`size` 就是订单，限价单照常可用
- [x] 冲突规则默认值：限价单的两个（同方向 `keepignore`、反方向撤单并执行）和 PRO 文档一致；PRO 文档没写止损冲突的默认值，保持参考实现的行为（止损替换信号）
- [ ] 限价单语义评审（见 [§4](#4-待决定)）

### 2.4 止损

- [x] `sl_stop`、`tp_stop`、`sl_trail`
- [x] 移动止损 `tsl_stop`、`tsl_th`
- [x] 时间止损 `td_stop`、`dt_stop`：按行数，或 `time_delta_format="index"` 时按时间（时长；时刻、周期末如 `"ME"`、每天的某个时刻如 `"18:00"`）
- [x] 止损距离按 `delta_format` 读：`percent`（默认）、`percent100`、`absolute`（价格差）、`target`（价位本身）；移动止损的门槛 `tsl_th` 同样
- [x] 止损阶梯 `stop_ladder`：`uniform`、`weighted`、`adaptuniform`、`adaptweighted`（后两种按触发时的持仓分）；`sl_stop`、`tsl_stop`、`tp_stop` 都可以是阶梯
- [x] 止损限价单 `stop_order_type="limit"`、`stop_limit_delta`，时间止损也可以（限价从开盘价算）
- [x] `upon_stop_exit`（含反手，PRO 的名字 `stop_exit_type` 也接受）、`stop_entry_price`、`stop_exit_price`、`upon_stop_update`
- [x] 不经模拟生成止损出场 `generate_ohlc_stop_exits`，支持阶梯（每级一次出场，`out["size"]` 是平掉的份额）、止损限价单和 `delta_format`；随机行情上和 `from_signals` 逐笔相同
- [x] 动态阶梯：`adjust` 改止损距离和价位，`signals` 在 `accumulate` 下按数量部分出场（测试：每涨 2% 减三分之一，止损上移到上一级）

### 2.5 账户与资金

- [x] 共享现金和分组（`cash_sharing`、`group_by` 对应 `group_lens`）
- [x] 入金出金 `cash_deposits`（开盘入账）
- [x] 现金收支 `cash_earnings`（收盘入账）
- [x] 分红 `cash_dividends`
- [x] 现金流记录 `CashRecords`：分析层只从记录重建余额，入金不算收益
- [x] 杠杆 `leverage`、`leverage_mode`（`lazy`/`eager`），做空占用按杠杆换算
- [x] 合约乘数 `multiplier`
- [x] 一根 K 线内的先后顺序：开盘现金流 → 保证金结算和强平检查 → 订单 → 收盘收支和持仓费
- [x] 公开的列状态 `ColumnState`
- [x] 初始持仓 `init_position`、`init_price`（`NaN` 是第一根收盘价）：记为第一根 K 线上按 `init_price`、无手续费的成交，经过内核，所以负债、抵押和保证金都是内核算的。现金列另记一条 `InitialPosition` 现金流（判别值 8）抵掉成交的现金，现金仍是 `init_cash`，收益把它当作投入的资本（同入金）；保证金列是一笔未平的交易，保证金从可用现金出。不为初始持仓布置止损
- [x] `init_cash="auto"`/`"autoalign"`（Python）：先用无限现金跑一遍，每个账户（`autoalign`：所有账户）取从记录算出的最大现金缺口，再跑一遍；只接受和现金无关的仓位类型、没有杠杆的现金账户，两次成交不同就报错
- [x] 负价格 `allow_negative_price`（默认关）：按单位下单；现金列可以买入、卖出持有的，保证金列多空都可以（持仓成本可以小于 0）；手续费、滑点、保证金按价格的绝对值算，滑点总是对交易不利；交易收益除以入场价值的绝对值。记录带上这个标志，交易视图才接受 0 及以下的价格，参考实现的 fixture 不变。百分比止损距离、持仓费和强平仍按正价格设计
- [x] 期货连续合约 `phasor.futures`：`roll_by_volume`、`roll_by_expiry`、`rolls`、`continuous`（差值或比例复权）；常用合约的乘数和最小变动价位 `CONTRACTS`、`spec_params`（保证金由交易所调整，不内置）
- [x] 外汇报价货币换算 `phasor.fx.convert`、`cross`：按每根 K 线的汇率把价格换成账户货币。对用账户货币买入的资产是精确的；对按报价货币结算的合约是近似（文档写明）。多币种现金账户不做（PRO 也没有）

### 2.6 保证金、强平、持仓费

➕ PRO 没有。phasor 原生支持，因为永续资金费套利、期货 CTA、外汇和 CFD 隔夜持仓、融资融券的结果主要取决于这几项。

- [x] 保证金账户 `account_type="margin"`、`initial_margin`、`margin_basis`（`notional`/`unit`）；未实现盈亏每根 K 线结算进现金；现货列和保证金列可以同组
- [x] 全仓、逐仓 `margin_mode`
- [x] 维持保证金和强平：按 K 线路径求强平价，跳空按开盘价，`liquidation_fee`；穿仓差额记为核销记录
- [x] 止损和强平的先后
- [x] 持仓费：`long_financing_rate`、`short_financing_rate`、永续 `funding_rate`
- [x] 标记价格 `mark_price`（持仓费和强平用）
- [x] 借款利息 `debt_rate`、借券费 `borrow_rate`
- [x] 年化费率换算 `phasor.rates.per_bar`
- [x] 按种类取现金流 `pf.cash_flows(kind)`
- [x] 手算场景测试（约七十个）和守恒性质测试
- [x] 和 NautilusTrader 1.231.0 的 `MarginAccount` 对照：线性永续按随机计划下市价单（含反手），4 组共 500 根 K 线、176 笔成交，每根 K 线的权益和每笔手续费都一致（`tests/parity/generate_nautilus_margin.py` 生成 `tests/fixtures/nautilus_margin.tsv`，`crates/phasor-analysis/tests/parity_nautilus.rs` 回放）
- [x] 借钱或借券的现金列（融资融券）的维持保证金 `cash_maintenance_margin`（名义价值比例，默认无）：跌破时和保证金列一起强平
- [x] 部分强平 `liquidation_step`：每次跌破平掉这个比例，沿 K 线剩下的路径继续检查，最多 64 次或剩下不到百万分之一时全平
- [x] 分档维持保证金 `maintenance_margin_tiers`（Python 里是 `(名义价值上限, 费率)` 列表，可每列一组）：`费率 × 名义价值 − 速算扣除数`，在档位边界连续；分档时用二分法求跌破点
- [x] 自动减仓 `auto_deleverage`：标记的 K 线上按标记价格（没有就用收盘价）平掉保证金列的持仓，不收手续费，记为强平；哪些 K 线由调用方给出（取决于交易所的风险保障基金，不是账户本身）
- [x] 逐仓列的资金费和隔夜融资从它自己的保证金扣，不动可用现金；分析层重建可用现金时同样处理（性质测试对照模拟的日志）
- [x] 负现金计息：账户现金低于 0、超出现金列多头借款的部分，按第一列的 `debt_rate` 计息
- [x] Python 的 `margin_basis` 按列设置

### 2.7 多周期与交易时段

- [x] 粗周期信号对齐到细周期 `phasor.resolution.align`；细周期序列聚合回粗周期 `resample`
- [x] 交易时段 `phasor.sessions.session_mask`、`session_end`：交易所时区、夏令时、隔夜时段
- [x] 强制平仓 `force_exit`（两个入口）
- [x] 只保留每个品种都有 K 线的时刻 `BarPanel.drop_missing()`
- [x] tick 数据：当成 K 线直接跑

### 2.8 状态延续与增量回测

验收：任意切点分两段跑再拼接，结果和一次跑完逐位一致，用随机切点测试。

- [ ] 状态快照 `SimState`（列、账户组和回调状态），`Portfolio.last_state`；公开的账户组状态 `GroupState`（现金、可用现金），可序列化
- [ ] 增量更新 `pf.update`：从快照继续，只算新增 K 线
- [ ] 分段拼接 `row_stack(chained=True)`
- [ ] 持久化 `save`/`load`：记录存 Arrow IPC 或 Parquet，状态存带版本号的二进制
- [ ] Stepper：外部执行层回报成交，phasor 逐根推进并对账
- [ ] 长历史分批，控制内存

### 2.9 数据

- [x] 读取本地 CSV、Parquet 和目录：`phasor.data` 的 `Source`、`Catalog`、`Dataset`、`BarPanel`
- [x] 按可用时间对齐 `asof`、窗口聚合 `window`
- [x] 合成数据 `random_walk`、`gbm`、`ohlc_from_close`（带种子，某列结果不随列数变化）
- [ ] 联网代码的包结构（见 [§4](#4-待决定)）
- [ ] 数据源第 1 批：Binance（现货、U 本位、币本位 K 线，资金费率历史，标记价格，指数价，Vision 批量文件）、CCXT
- [ ] 资金费率历史接进 `research/phasor_lab`，`funding_modeled` 改成真实开关
- [ ] 数据源第 2 批：Yahoo Finance、Alpaca、Polygon、Databento
- [ ] 数据源第 3 批：Alpha Vantage、Nasdaq Data Link、TradingView、findatapy
- [ ] 自定义数据源基类
- [ ] 存储：DuckDB、SQL、ArcticDB
- [ ] 数据增量更新
- [ ] 带 `available_time` 的 OHLCV 重采样
- [ ] 打乱或扰动已有序列 `randomize`

### 2.10 指标

- [x] 8 个 Rust 指标（MA、MSTD、BBANDS、RSI、STOCH、MACD、ATR、OBV）和参数扫描
- [x] 因子表达式 DSL，`FactorSet`，Alpha158、Alpha360
- [ ] Rust 指标接到 Python
- [ ] 指标工厂：自定义指标做参数网格、带参数标签输出
- [ ] 第三方库包装：TA-Lib、pandas-ta、ta、smartmoneyconcepts
- [ ] 截面算子（`CSRank`、`CSZScore`、`Scale`、中性化），按图分阶段执行
- [ ] WorldQuant 101，对照公开公式手算 fixture
- [ ] 流式指标：逐根更新，结果和批量计算一致
- [ ] 因子输出分批写到 sink 或 Arrow/Parquet，峰值内存有界
- [ ] 更多逐元素算子：`Exp`、`Sqrt`、`Clip`、`FillNa`、`IsNan`、掩码
- [ ] 窄面板上单标的内部并行

### 2.11 信号与标签

- [x] 带种子随机信号 `random_by_prob`、`random_n`、`random_entries_exits`
- [x] 信号区间和长度：`partition_ranges`、`between_ranges`、`duration` 等（Rust）
- [x] ML 标签 `fixed_horizon_returns`、`trend_labels`、`triple_barrier`、`pivot_labels`
- [ ] 信号清洗和区间函数接到 Python
- [ ] 按日历生成信号（每月第一个交易日等）
- [ ] 信号汇总统计表（次数、间隔和长度的均值与分位数）
- [ ] 匹配型随机基线：信号数量和分布与原策略相同，只打乱位置
- [ ] 均值类标签
- [ ] 用模型预测值回测并和简单基线比较

### 2.12 组合分析

- [x] 订单、日志、交易（三种视图）、持仓记录，mapped array
- [x] 价值和收益重建，收益和风险指标，基准对比，统计表，按层级聚合
- [x] 回撤、滚动指标（Rust）
- [x] 敞口：`allocations`、`long_exposure`、`short_exposure`、`net_exposure`、`gross_long_short`、`position_coverage`
- [x] MAE/MFE、edge ratio
- [x] 事件研究 `phasor.events`
- [ ] 回撤记录、滚动指标接到 Python
- [ ] MAE/MFE 多空分开统计，加进统计表
- [ ] 外部收益序列分析：Python 入口，以及不需要交易记录的统计表
- [ ] 组合切片、拼接、重采样（含 `Portfolio.resample`）
- [ ] 形态搜索
- [ ] 图表：可选 extra `phasor[plot]`（Plotly）
- [ ] 结果转 pandas

### 2.13 参数优化与验证

- [x] 策略参数扫描：`@strategy` 的 `sweep` 接受参数组合列表，按列平铺一次跑完；`signal_delay`
- [x] 交叉验证切分：`rolling`、`expanding`、带 purge 和 embargo 的 `purged_kfold`、`walk_forward`、自定义 `Split`
- [x] 稳健性：`probabilistic_sharpe`、`deflated_sharpe`、`expected_max_sharpe`、`pbo_cscv`
- [ ] 通用参数化：任意 Python 函数按网格、条件网格、随机搜索扫参数，结果带参数标签
- [ ] 从日期区间或掩码生成切分，含按日历切分
- [ ] `research/phasor_lab` 的 walk-forward 改用 `phasor.validation`（先处理不完整窗口、零波动和 NaN、重跑还是切片）
- [ ] 置换检验
- [ ] 邻近参数比较
- [ ] 延迟信号检验：并排比较 `signal_delay=k` 和 `k+1`
- [ ] 在打乱的价格上重跑（依赖 `randomize`）
- [ ] 组合优化：自定义权重函数和调仓计划；PyPortfolioOpt、Riskfolio-Lib、Universal Portfolios 包装；权重转 `from_orders(targetpercent)`；等权基线

### 2.14 性能与分布式

- [x] 账户组多线程，串行和多线程逐位一致
- [x] 功能关闭时的回归检查：`benchmarks/instructions.py`（callgrind 指令数，超过 3% 失败）
- [ ] 分块执行和断点续跑
- [ ] 多进程和 Ray，结果和单进程一致
- [ ] 跨运行缓存（先测复用率）
- [ ] 新功能写进 `benchmarks/workloads*.tsv`，开启和关闭各测一次

### 2.15 工具链

- [ ] 配置文件（TOML 描述一次回测）
- [ ] 研究对象持久化（复用 2.8 的 `save`/`load`）
- [ ] 定时任务和通知（Telegram、Slack）
- [ ] CLI：`phasor run`、`phasor data update`、`phasor sweep`
- [ ] 开发者工具：`phasor.help(obj)`，检查广播后的输入形状

### 2.16 AI 接口

- [ ] 文档检索
- [ ] MCP 服务：查 API、读源码、跑小回测；不能接到 `research/phasor_lab` 的数据和账本（模型不能看到研究结果）
- [ ] Agent skills：怎么写策略、怎么加功能、怎么跑对照测试

### 2.17 工程维护

不对应 PRO 功能，是已知问题。都不影响结果的正确性。

- [x] `from_signals` 性能回到约束以内（拆出 `FLOWS`、`LEV` 实例，内核冷路径拆分）
- [x] 新功能接到 Python
- [x] 合并三份 SplitMix64 实现到 `phasor_core::random`
- [ ] `from_orders` 指令数比 c0fc6b2 累计多约 4.4%（价格选项 +2.04%、百分比仓位 +1.24%、负价格的分派约每根 K 线 1 条指令 +0.79%），压回 3% 以内，并补一次交替计时
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc` 不通过：`phasor-sim`、`phasor-records` 的文档链接指向私有模块或其他 crate
- [ ] `Symbol` / `Venue` 未驻留，但模块注释说已驻留（`phasor-model`）
- [ ] 过时注释：`phasor-analysis` 说不支持共享现金；`from_orders.rs` 用镜像不可用来解释作用域线程
- [ ] `propose.py` 引用 `tests/test_propose.py`，实际检查在 `research/tests/test_pipeline.py::test_proposals_never_see_results`
- [ ] 两个 Qlib fixture 用了不同版本的 Qlib/NumPy/pandas（见 fixture 文件头）

---

## 3. 约束与验证

所有改动都要遵守，违反了要当作缺陷处理。

| 约束 | 内容 |
|---|---|
| 默认行为不变 | 新参数取默认值时，结果和之前逐位一致。`tests/fixtures/`（vectorbt 1.1.0 和 Qlib）不因新功能重新生成 |
| 关闭时不变慢 | 新功能全部关闭时，`benchmarks/instructions.py` 的指令数几何平均增加不超过 3%；计时用 `cargo run --release --example bench -p phasor-sim` 和改动前交替多轮，看中位数 |
| 不抄代码 | PRO 闭源，只按公开文档的行为独立实现，参数名尽量沿用（见 README 的 Licence） |
| 记录是唯一来源 | 分析层只从记录重建余额。改变现金或持仓的东西都要落成记录 |
| 库不联网 | `crates/` 下的库既不下载也不写文件 |
| 可并行 | 不用 `Rc`、`RefCell` 或进程级全局状态；模拟状态按列或按账户组存放 |
| 线程数不影响结果 | 串行和多线程输出逐位一致，每个新功能都要测 |

PRO 闭源，没法差分测试，按功能来源对照：

| 类别 | 例子 | 对照方式 |
|---|---|---|
| vectorbt 开源版也有 | `init_cash="auto"`、部分止损选项 | 外部环境运行 vectorbt 1.1.0，生成 TSV fixture |
| 只有 PRO 有 | 杠杆、限价单、时间止损、增量更新 | 手算场景加性质测试 |
| PRO 也没有 | 保证金、强平、持仓费 | 手算场景、性质测试，NautilusTrader 对照（`tests/fixtures/nautilus_margin.tsv`） |

性质测试：退化一致（默认参数逐位不变）、守恒（现金和净值能从记录重建）、线性（`multiplier = k` 等于数量乘 k）、切分不变（续跑等于一次跑完）、线程不变（1、3、N 线程逐位一致）、变异测试（去掉一处关键逻辑，测试必须失败）。

### 性能记录

功能全关时和改动前比，单线程，callgrind 指令数几何平均。

| 改动 | 对比版本 | `from_orders` | `from_signals` |
|---|---|---:|---:|
| 现金流、`cash_limit`、`reject_prob` 修复后 | main | +0.5% | −4.5% |
| 杠杆 | d0c23d1 | +1.43% | +1.06% |
| 保证金账户 | d79f0f5 | −2.60% | +0.57% |
| 持仓费、标记价格、借贷利息 | dbf2625 | +0.73% | +0.09% |
| 强平 | 2b05c36 | −0.58% | +0.43% |
| 合约乘数 | c2bad21 | +0.29% | +0.43% |
| 限价单 | c0fc6b2 | +0.15% | +1.02% |
| 挂单冲突 | 1aea813 | +0.00% | −2.19% |
| 价格选项、止损扩展 | 8d09d8f | +2.04% | −0.03% |
| 不经模拟的止损出场、目标仓位、订单模式 | ab4674c | +0.00% | +0.29% |
| 百分比仓位、多周期、交易时段 | e9d8d59 | +1.24% | +0.04% |
| 按根的 `from_ago`、显式调用顺序、止损的 `delta_format` 和阶梯、按时间的止损和有效期 | e94e740 | +0.14% | −0.37% |
| 初始持仓、负价格 | eaabcc2 | +0.79% | +1.23% |
| 融资融券强平、分档、部分强平、ADL、逐仓持仓费、负现金计息 | 40a3e79 | −0.42% | −0.47% |

从限价单起只测了指令数，没有跑交替计时。单次都在 3% 以内，但累计起来，和 c0fc6b2 比 `from_orders` 在 e94e740 时是 +3.47%（`from_signals` −0.90%），之后又加了约 0.9 个百分点，超过约束，见 2.17。

---

## 4. 待决定

| 问题 | 建议 |
|---|---|
| Python 回调方式 | 直接调用已做（调试模式，每列每根 K 线拿一次 GIL）。Rust 插件就是 `OrderFunc`/`SignalFunc`。待定：常见回调（按风险定仓位、移动止损到保本、全局预算）做成表达式 DSL 还是做成内置回调加参数 |
| 联网代码放哪里 | 单独的可选 Python 包（如 `phasor-connectors`），保持库不联网；数据库读写放 `phasor-store` 或 Python 层 |
| 能否用 PRO 的输出做对照 | 先确认 PRO 许可条款 |
| 负价格下 `percent` 仓位和收益率的语义 | 先写说明，评审后实现 |
| 图表依赖 | Plotly，作为可选 extra |
| 限价单语义（已按此实现，待评审） | 每列每根 K 线只执行一笔：已触发的止损 > 本根被触及的挂单 > 本根新信号。挂单按挂出时的持仓定数量，本列有其他成交就撤单。跳空越过限价按开盘价（`limit`）、限价（`hardlimit`）或收盘价（`close`）成交，不加滑点 |
| 其他市场需要、计划里没有的功能 | 多币种账户、币本位（反向）合约、期货换月、A 股 T+1 和涨跌停、期权。有需要时单独立项 |
