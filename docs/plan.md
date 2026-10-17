# phasor 功能计划：对齐 VectorBT PRO

目标：实现 VectorBT PRO（下称 PRO）公开文档列出的全部功能，并补上 PRO 没有的保证金账户、强平和持仓费。
功能清单来自 2026-10 对 [vectorbt.pro/features](https://vectorbt.pro/features/overview/) 的整理；PRO 没写参数名的功能只描述行为。

状态以代码为准。最后核对：2026-10-16，分支 `claude/dreamy-dijkstra-39adnh`。

图例：✅ 完成　🟡 部分完成　⬜ 未开始　➕ PRO 没有、phasor 有

---

## 1. 功能总表

| # | 功能 | 状态 | 已有 | 还缺 |
|---|---|:-:|---|---|
| 1 | [模拟入口与回调](#21-模拟入口与回调) | ✅ | `from_orders`、`from_signals`（含 `order_mode`）、`from_order_func`（flex、`row_wise`）、两个循环的回调钩子、Python 回调（直接调用）、内置回调（按风险定仓位、移到保本、全局预算）；内核、止损判定、挂单判定、现金流入账、记录写入已公开 | — |
| 2 | [订单与仓位](#22-订单与仓位) | ✅ | 全部仓位类型、下单约束、价格选项（含 `nextvalidopen`、按根的 `from_ago`）、买卖价、`call_seq`（auto 按现金影响、显式顺序） | — |
| 3 | [限价单与挂单](#23-限价单与挂单) | 🟡 | 限价单、突破单、有效期（按行数或按时间）、挂单冲突 | 语义评审 |
| 4 | [止损](#24-止损) | ✅ | 止损、止盈、移动止损、时间止损（按行数或按时间）、四种阶梯、`delta_format`、止损限价单、不经模拟的止损出场、回调里的动态阶梯 | — |
| 5 | [账户与资金](#25-账户与资金) | ✅ | 共享现金、入金出金、收支、分红、杠杆、合约乘数、初始持仓、`init_cash="auto"`、负价格、连续合约、汇率换算 | — |
| 6 | [保证金、强平、持仓费](#26-保证金强平持仓费) ➕ | ✅ | 全仓和逐仓保证金、强平（含部分强平、分档、ADL、融资融券）、资金费、隔夜融资、借贷利息、标记价格；和 NautilusTrader 对照一致 | — |
| 7 | [多周期与交易时段](#27-多周期与交易时段) | ✅ | 粗周期对齐到细周期、聚合回粗周期、交易时段、强制平仓 | `Portfolio.resample`（归在组合分析） |
| 8 | [状态延续与增量回测](#28-状态延续与增量回测) | ✅ | 状态快照 `SimState`、续跑（`resume=pf`）、分段拼接、长历史分批（`row_offset`）、`save`/`load`、`Stepper`（回报成交、对账）；任意切点逐位一致 | — |
| 9 | [数据](#29-数据) | 🟡 | 本地 CSV、Parquet、目录读取，按可用时间对齐，合成数据 | 数据源连接器、数据库存储、增量更新 |
| 10 | [指标](#210-指标) | ✅ | 8 个 Rust 指标和参数扫描、流式版本，Python 接口、指标工厂、第三方库；因子 DSL（Alpha158、Alpha360、WorldQuant 101），截面算子分阶段执行，分批输出，窄面板按因子并行 | — |
| 11 | [信号与标签](#211-信号与标签) | 🟡 | 带种子随机信号，信号区间，4 种 ML 标签 | Python 接口、日历信号、统计表 |
| 12 | [组合分析](#212-组合分析) | 🟡 | 记录、收益与风险指标、回撤、敞口、MAE/MFE、事件研究 | 切片拼接重采样、图表、形态搜索、部分 Python 接口 |
| 13 | [参数优化与验证](#213-参数优化与验证) | 🟡 | 策略参数扫描、通用参数化（网格、条件网格、随机搜索）、邻近参数比较；交叉验证切分（含按日期、掩码、日历）；PSR、deflated Sharpe、PBO、置换检验、延迟信号检验、打乱价格重跑；组合优化（权重函数、调仓计划、第三方库包装、权重转订单） | `research/phasor_lab` 改用 `phasor.validation`（待决定） |
| 14 | [性能与分布式](#214-性能与分布式) | ✅ | 账户组多线程（逐位一致），指令数回归检查；按行分块和断点续跑，按列分到多进程或 Ray（逐位一致），跨运行缓存（带命中率统计），新功能的开关对照工作负载 | — |
| 15 | [工具链](#215-工具链) | 🟡 | TOML 配置、CLI（`phasor run/sweep/schedule/help`）、定时任务和通知、研究对象持久化、`phasor.help`、`check_shapes` | `phasor data update`（依赖数据层的增量更新） |
| 16 | [AI 接口](#216-ai-接口) | ✅ | 文档检索 `phasor.docs`、MCP 服务 `phasor mcp`（不能读 `research/`）、3 个 agent skills | — |

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
- [x] 常见回调做成内置回调加参数（不做表达式 DSL，见 [§4](#4-待决定)）：`phasor_sim::Builtin`，Python `from_signals(risk=, risk_stop=, breakeven=, max_positions=, max_exposure=)`，在 Rust 里跑、不拿 GIL，和 Python 回调不能同时用。按风险定仓位（开仓或反手时数量使 `risk_stop` 处止损亏 `risk` 的净值）；移到保本（上一根收盘浮盈达 `breakeven` 后把止损移到入场价，不放松更紧的止损，触发只读上一根，不偷看）；全局预算（持仓数达 `max_positions`，含同一根已放行的，或总敞口达 `max_exposure` 倍净值时丢掉开新仓的信号，平仓和反手不受限）。测试：和写成 Python 回调的同一规则逐笔一致；1 和 3 线程一致；5 个切点续跑和一次跑完一致

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

- [x] 状态快照 `phasor_sim::snapshot::SimState`：每个账户组的现金、可用现金、各列状态（持仓、债务、锁定现金、估值价、止损、挂单）、是否被回调停止；`Portfolio.last_state`；带版本号的二进制 `to_bytes`/`from_bytes`。行号存绝对行号。回调自己的状态不在里面：续跑时再调一次 `pre_group`
- [x] 续跑（即 `pf.update`）：两个循环的参数加 `resume`；Python `from_signals(..., resume=pf)`、`from_orders(..., resume=pf)` 传入完整历史，只算新增 K 线，返回整段历史的组合；核对旧 K 线的收盘价、分组和列
- [x] 分段拼接：`OrderRecords::chain`、`Journal::chain` 按账户组稳定排序，结果和一次跑完的记录顺序完全相同（日志的 `order_id` 跟着重编）；Python 的 `resume=pf` 直接返回拼好的组合
- [x] 持久化：`Portfolio.to_bytes`/`from_bytes`，`phasor.save`/`phasor.load`（Python 写文件，库不写文件）。没有用 Arrow IPC，用的是带版本号的二进制（记录、日志、分析需要的输入、状态）；回调状态 `in_outputs` 不保存
- [x] Stepper：`phasor.Stepper` 逐根 `propose`（回测会怎么成交）、`commit(fills)`（记下券商回报的数量、价格、费用，持仓费和现金流照常），`reconcile(positions)` 对账；每根只跑这一根和回看的几根，成本不随历史变长。只支持 `from_orders`
- [x] 长历史分批：参数 `row_offset`，数组从历史的任意一行开始，带上回看需要的几行；Rust 测试 7 行一段跑完 90 行；Python `resume=SimState, row_offset=` 返回 `phasor.Segment`
- 验收：`crates/phasor-sim/tests/resume.rs`，两个循环各 4 组功能（共享现金、杠杆、保证金和强平、止损含时间止损、限价单、随机拒单、现金流、持仓费）× 5 个切点 × 两种续跑方式，记录和最终状态逐位一致；去掉时间止损的行号换算或随机数改用局部行号，测试失败

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
- [x] Rust 指标接到 Python：`phasor.indicators.ma/mstd/bbands/rsi/stoch/macd/atr/obv/true_range`，参数列表做乘积（或 `product=False` 逐位配对），`IndicatorResult` 带每列参数标签和 `select(...)`
- [x] 指标工厂：`IndicatorFactory(func, inputs, params, outputs).run(...)`，任意 Python 函数按参数网格跑，输出布局和内置指标相同
- [x] 第三方库包装：`from_talib`、`from_pandas_ta`、`from_ta`、`from_smc`，用到时才导入；TA-Lib 的 SMA 和内置 MA 相差 2e-13（测试在装了 TA-Lib 时跑）
- [x] 截面算子：`CSRank`（pandas `rank(pct=True)`）、`CSZScore`、`CSDemean`、`Scale(x, a)`、`IndNeutralize(x, $group)`（组内去均值）、`CSResi(y, x)`（截面回归残差）。图按截面节点分层（`phasor_expr::stage`）：同层的逐标的节点编成一个普通图按列并行，层间对每行跑截面节点；跨层的序列存成整块面板，最后一个读者用完就释放。线程数不影响结果（测试 1、2、4、7 线程逐位一致）
- [x] WorldQuant 101：`wq101()`（只读量价的 82 个）、`wq101_all()`（加 `$cap`、`$sector`、`$industry`、`$subindustry` 共 101 个）、`wq101_formula(k)`。公式从论文原文（arXiv:1601.00991v3 附录 A）机械翻译，`tests/parity/generate_wq101.py` 同时生成 Rust 表和 fixture；fixture 由一个独立的 NumPy 解释器按论文定义逐个算子直接计算。精确算术下相等、浮点下差几个 ulp 的值（两点相关系数恰为 ±1、秩序列的协方差）会让排名两边都对但不同，解释器跟踪这类近似并列，fixture 标出这些格子不比较（约 6%）；其余格子相对误差 1e-9 内一致。`adv{d}` 取 `Mean($volume, d)`（论文说是成交额，但多个 alpha 直接拿它和 `volume` 比）
- [x] 流式指标：`phasor_indicators::Stream`（Rust）、`phasor.indicators.Stream`（Python），8 个指标逐根更新，状态是批量算法跨根携带的那部分，浮点运算顺序相同，结果和批量计算逐位一致（含缺失值、常数段、`adjust` 两种取法）
- [x] 因子分批输出：`FactorSet::evaluate_batches(inputs, batch, sink)` 每次算 `batch` 个标的交给 sink，没有截面算子时峰值内存只和批大小有关；有截面算子时先整块算完再分批交出。Python `FactorSet.evaluate_batches`、`FactorSet.to_parquet`（pyarrow，每批一个 row group）
- [x] 更多逐元素算子：`Exp`、`Sqrt`、`Clip(x, lo, hi)`、`FillNa(x, v)`、`IsNan`、`Mask(x, cond)`、`SignedPower`；时间序列 `DecayLinear`、`Prod`
- [x] 窄面板并行：标的数少于线程数时，因子按顺序切成至多 16 组分别编译，按（组、标的）并行；共享子表达式在组间会重复计算，结果逐位不变。单标的 Alpha158（5 万行）1 线程 111 ms，4 线程 40 ms

### 2.11 信号与标签

- [x] 带种子随机信号 `random_by_prob`、`random_n`、`random_entries_exits`
- [x] 信号区间和长度：`partition_ranges`、`between_ranges`、`duration` 等（Rust）
- [x] ML 标签 `fixed_horizon_returns`、`trend_labels`、`triple_barrier`、`pivot_labels`
- [x] 信号清洗和区间函数接到 Python：`phasor.signals.clean`、`exit_after`、`exit_on`、`ranges`、`rank`、`nth_index`、`normalized_average_index`
- [x] 按日历生成信号：`calendar(index, "ME", which="first"|"last", n=)`，按实际有的 K 线数（每月第一个交易日就是当月第一根）
- [x] 信号汇总统计表：`signals.stats`（次数、频率、首末位置、间隔和连续长度的均值、中位数、10% 和 90% 分位、平均位置）
- [x] 匹配型随机基线：`random_like(keep="count")` 保持每列次数，`keep="gaps"` 还保持间隔分布，只打乱顺序和起点
- [x] 均值类标签：`labels.mean_labels(close, window, wait=)`
- [x] 用模型预测值回测并和基线比较：`phasor.predictions.backtest`（模型、一直持有、匹配随机信号）和 `compare`

### 2.12 组合分析

- [x] 订单、日志、交易（三种视图）、持仓记录，mapped array
- [x] 价值和收益重建，收益和风险指标，基准对比，统计表，按层级聚合
- [x] 回撤、滚动指标（Rust）
- [x] 敞口：`allocations`、`long_exposure`、`short_exposure`、`net_exposure`、`gross_long_short`、`position_coverage`
- [x] MAE/MFE、edge ratio
- [x] 事件研究 `phasor.events`
- [x] 回撤记录、滚动指标接到 Python：`pf.drawdowns()`（含派生的回撤幅度、持续、下跌和恢复时长）、`pf.rolling(metric, window)`（18 个指标，含对买入持有的 beta、alpha 等）
- [x] MAE/MFE 多空分开统计：`pf.stats(high=, low=)` 加 `avg_mae_long`、`avg_mfe_long`、`edge_ratio_long` 和空头三项
- [x] 外部收益序列分析：`phasor.returns.metric/rolling/drawdowns/stats/value`，和组合用同一套 Rust 指标，结果一致
- [x] 组合切片、拼接、重采样：`Portfolio.from_records` 从记录建组合；`phasor.portfolios.select`（按账户）、`column_stack`、`row_slice`（从切点前的现金和持仓接着算，不支持杠杆和保证金）、`resample`（`resample_bars(index, "W")`）；日志不带过去
- [x] 形态搜索：`phasor.patterns.search`（滑动窗口，min-max 或 z-score 缩放，可多个窗口长度，默认不重叠），`patterns.signals` 在形态完成后一根给信号
- [x] 图表：`phasor.plotting`（净值和回撤、成交标记、最深回撤、滚动指标），可选 extra `phasor[plot]`
- [x] 结果转 pandas：`phasor.frames`（记录、净值、收益、统计表），可选 extra `phasor[pandas]`

### 2.13 参数优化与验证

- [x] 策略参数扫描：`@strategy` 的 `sweep` 接受参数组合列表，按列平铺一次跑完；`signal_delay`
- [x] 交叉验证切分：`rolling`、`expanding`、带 purge 和 embargo 的 `purged_kfold`、`walk_forward`、自定义 `Split`
- [x] 稳健性：`probabilistic_sharpe`、`deflated_sharpe`、`expected_max_sharpe`、`pbo_cscv`
- [x] 通用参数化 `phasor.params`：`grid(..., where=)`（条件网格）、`random(n, seed=, a=IntUniform(...), b=LogUniform(...), c=[...])`（随机搜索，同种子同结果）、`run(func, combos, n_jobs=, executor="thread"|"process")`，结果 `ParamResults` 按网格顺序，带参数标签：`table()`、`to_frame()`、`best()`、`select()`
- [x] 从日期区间或掩码生成切分：`validation.from_ranges(index, [(train_start, train_stop, test_start, test_stop)])`、`from_masks`；按日历：`validation.calendar(index, "M", train=6, test=1, expanding=)`（周、月、季、年，按实际有 K 线的周期）
- [ ] `research/phasor_lab` 的 walk-forward 改用 `phasor.validation`：没做。lab 的选择规则是预注册协议的一部分，换实现会改变已登记 spec 的结果，要由 lab 的维护者决定（见 [§4](#4-待决定)）
- [x] 置换检验 `validation.permutation_test(positions, returns, statistic=, n=, block=)`：按块打乱收益、保留持仓时间，p 值 `(1 + #null ≥ observed) / (n + 1)`
- [x] 邻近参数比较 `params.neighbors(results)`：每个点和网格上相邻点（某一参数差一档）的均值、最小值、比值
- [x] 延迟信号检验 `validation.signal_delay_test(strategy, data, params, delays=(0, 1, 2))`：同一策略各个 `signal_delay` 的指标并排
- [x] 在打乱的价格上重跑：`validation.shuffled_prices`（按块打乱对数收益，所有列同一排列，保留截面相关）、`shuffled_test(fn, close, n)`。放在验证层，不依赖数据层的 `randomize`
- [x] 组合优化 `phasor.allocation`：权重函数（`equal_weight`、`inverse_volatility`、`min_variance`、`max_sharpe`、`risk_parity`，或任意函数）只看调仓 K 线之前的收益；调仓计划 `rebalance_mask(every=|freq=)`、`weights(close, fn, every=, lookback=)`；`from_pypfopt`、`from_riskfolio`、`universal_weights` 包装，用到时才导入；`from_weights` 转 `from_orders(size_type="targetpercent")`，共享现金、先卖后买；等权基线就是 `weights(close, equal_weight, ...)`

### 2.14 性能与分布式

- [x] 账户组多线程，串行和多线程逐位一致
- [x] 功能关闭时的回归检查：`benchmarks/instructions.py`（callgrind 指令数，超过 3% 失败）
- [x] 分块执行和断点续跑：`phasor.chunked.run("from_signals" | "from_orders", close, ..., chunk=, lookback=, checkpoint=path)`，每块用 `resume=SimState, row_offset=` 接上一块，记录按账户稳定排序后和一次跑完逐位一致（块长 1、37、100、300 测过）；每块后写检查点（状态和已有记录），中断后同一调用从下一块继续，检查点来自别的形状或块长时报错。结果用 `Portfolio.from_records` 重建，不支持杠杆和保证金账户的分析
- [x] 多进程和 Ray：`phasor.distributed.run(..., n_chunks=, backend="process" | "thread" | "ray" | "serial")` 按整个账户组切列，每列的数组跟着切，进程间用 `Portfolio.to_bytes` 传，`portfolios.column_stack` 拼回；共享现金和不共享都和单进程逐位一致（Ray 未装，没有在 Ray 上跑过）
- [x] 跨运行缓存：`phasor.cache.Cache(directory, max_items)`，`memoize` 按函数和参数的摘要（数组按字节、形状、dtype）缓存，内存 LRU 加可选磁盘，`stats()` 给命中、磁盘命中、未命中和命中率，用来先测复用率；组合存为版本化字节
- [x] 新功能的开关对照：`benchmarks/workloads_signals.tsv` 加 `features` 列和 4 行（`limit`、`leverage`、`margin`、`timestop`），都和形状相同、功能关闭的 `signals_both` 比。单线程，整个进程（含生成数据）交替跑 7 轮的中位数：`signals_both` 127 ms，限价单 +42%，杠杆 +11%，保证金账户加资金费 +187%（每根结算和计费），时间止损 +68%

### 2.15 工具链

- [x] 配置文件 `phasor.config`：TOML 的 `[data]`（`.npz`、长格式 CSV 或自定义 `loader = "模块:函数"`，不经数据层）、`[signals]`（因子表达式，`{name}` 占位）、`[backtest]`（`from_signals`/`from_orders` 的参数）、`[sweep]`（网格）、`[output]`（统计 JSON、组合文件）；`load` 检查节名、必需的信号、占位符都有取值
- [x] 研究对象持久化：`phasor.save`/`load` 除组合外接受任意研究对象（`ParamResults`、`IndicatorResult`、数组字典、组合列表），其中的组合按版本化字节存；其他用 pickle，只读可信来源
- [x] 定时任务和通知 `phasor.schedule`：`Job`（成功发摘要，失败发 traceback，通知失败不掩盖结果）、`every(job, at="06:30" | interval=)`、`crontab(config, at=, weekdays=)` 生成 cron 行；通知器是 `(subject, text)` 可调用对象，库里只有 `File` 和 `Command`（不联网），Telegram、Slack 在 `examples/notifiers.py`
- [x] CLI：`phasor run config.toml`、`phasor sweep config.toml --set window=10,20,50`、`phasor schedule`、`phasor help NAME`，`--notify-file`、`--notify-command`
- [ ] CLI `phasor data update`：没做，依赖数据层的增量更新（模块 9 暂不动）
- [x] 开发者工具：`phasor.help(obj 或 "indicators.rsi")`（签名、文档首段、成员）；`phasor.check_shapes(close, **inputs)` 按引擎的广播规则报告每个输入是标量、逐根、逐列还是整块，一次列出所有不合规的输入

### 2.16 AI 接口

- [x] 文档检索 `phasor.docs.search(query)`：已安装包里每个公开对象的文档字符串按段落建 BM25 索引（名字和标题加权，只收 phasor 自己的对象）；`Index(extra=[...])` 加上检出目录里的 Markdown，按标题切段；本地计算，不联网
- [x] MCP 服务 `phasor mcp [--source 检出目录]`（或 `python -m phasor.mcp`）：stdio 上的 JSON-RPC 2.0，协议版本 2025-06-18，不依赖 SDK。工具：`search_docs`、`api_reference`、`read_source`（phasor 对象的 Python 源码，或检出目录里 `crates/`、`docs/`、`examples/`、`skills/`、`benchmarks/`、`tests/`、`README.md`、`AGENTS.md` 下的文件）、`run_backtest`（合成 GBM 或内联的小数组，至多 20 万格，信号写成因子表达式）。读不到 `research/`（`phasor_lab` 的数据和账本）和检出目录以外的路径，回测不接受文件输入；测试覆盖这些拒绝
- [x] Agent skills：`skills/phasor-strategy`（写策略、扫参数、报告前的检验）、`skills/phasor-feature`（代码放哪里、约束、检查顺序、收尾）、`skills/phasor-parity`（重放和诊断对照测试、加覆盖、性质和变异测试），链接到 `.claude/`、`.agents/`、`.grok/`

### 2.17 工程维护

不对应 PRO 功能，是已知问题。都不影响结果的正确性。

- [x] `from_signals` 性能回到约束以内（拆出 `FLOWS`、`LEV` 实例，内核冷路径拆分）
- [x] 新功能接到 Python
- [x] 合并三份 SplitMix64 实现到 `phasor_core::random`
- [x] `from_orders` 指令数比 c0fc6b2 累计超过 3%：`FlexArray::get` 标量快路径后是 +2.47%（`from_signals` −14.30%）
- [x] 交替计时：c0fc6b2 和 aeb8f77 的 `bench` 交替各跑 5 轮，单线程，每个工作负载取中位数（成交数和校验和相同）：`from_orders` 几何平均 +1.14%（指令数 +2.47%），`from_signals` −20.66%（指令数 −14.30%）。标量快路径在时间上也是降的
- [x] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` 通过：指向私有模块和其他 crate 的链接改成代码字面量，`excursions` 的歧义链接指明是函数
- [x] `Symbol` / `Venue` 的模块注释改成实情（`Arc<str>`，未驻留）
- [x] 过时注释：`phasor-analysis` 支持共享现金的组；`from_orders.rs` 里已没有镜像的说法
- [x] `propose.py` 指向 `research/tests/test_pipeline.py::test_proposals_never_see_results`
- [ ] 两个 Qlib fixture 用了不同版本的 Qlib/NumPy/pandas（见 fixture 文件头）
- [ ] `cargo test -p phasor-expr --features polars --test parity_qlib` 在 debug 构建下多数次数栈溢出（polars 工作线程上，Alpha158 的表达式树深）；release 和原生后端不受影响。126d8da 上也一样

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
| 回调、`from_order_func`、`row_wise`；`FlexArray::get` 标量快路径 | ee45e7f | −1.18% | −13.85% |
| 状态快照、续跑、分批 | 6aa71f9 | +1.01% | +1.58% |

交替计时（2026-10-17，c0fc6b2 对 aeb8f77，5 轮中位数）和累计指令数一致：`from_orders` +1.14%，`from_signals` −20.66%。回调这一行是两件事相抵：回调本身让 `from_orders` 多了约 4%（循环拆成 `Group::run_rows` 以支持 `row_wise`，寄存器分配变差），`FlexArray::get` 对标量直接返回、不再调用函数，省回约 5%。累计和 c0fc6b2 比：`from_orders` +2.47%，`from_signals` −14.30%，回到约束以内。

---

## 4. 待决定

| 问题 | 建议 |
|---|---|
| Python 回调方式 | 已定：直接调用（调试模式，每列每根 K 线拿一次 GIL）；Rust 插件就是 `OrderFunc`/`SignalFunc`；常见回调做成内置回调加参数（`Builtin`），不做表达式 DSL |
| 联网代码放哪里 | 单独的可选 Python 包（如 `phasor-connectors`），保持库不联网；数据库读写放 `phasor-store` 或 Python 层 |
| 能否用 PRO 的输出做对照 | 先确认 PRO 许可条款 |
| 负价格下 `percent` 仓位和收益率的语义 | 先写说明，评审后实现 |
| 图表依赖 | Plotly，作为可选 extra |
| 限价单语义（已按此实现，待评审） | 每列每根 K 线只执行一笔：已触发的止损 > 本根被触及的挂单 > 本根新信号。挂单按挂出时的持仓定数量，本列有其他成交就撤单。跳空越过限价按开盘价（`limit`）、限价（`hardlimit`）或收盘价（`close`）成交，不加滑点 |
| `research/phasor_lab` 的 walk-forward 是否改用 `phasor.validation` | lab 维护者决定：换实现会改变已登记 spec 的结果。若换，先定不完整窗口、零波动、NaN 的处理，以及每个窗口重跑还是切片 |
| 其他市场需要、计划里没有的功能 | 多币种账户、币本位（反向）合约、期货换月、A 股 T+1 和涨跌停、期权。有需要时单独立项 |
