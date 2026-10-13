---
title: 术语表
kind: reference
summary: 知识库里反复出现的术语，各用一句话定义，并链接到详细解释它的页面。
related: [conventions, system-map, algorithm-index, further-reading]
tags: [glossary, reference]
sources:
  - docs/ARCHITECTURE.zh-CN.md
order: 40
---

# 术语表

知识库里反复出现的术语，各用一句话定义；"详见"一栏链接到解释它的页面。
定义取自 [ARCHITECTURE 附录 B](../../../docs/ARCHITECTURE.zh-CN.md#附录-b-术语表) 和各页面的正文，按所属的层分组。

## 数据布局

| 术语 | 含义 | 详见 |
|---|---|---|
| 列 | 一条被模拟的序列：一个标的，或一个"参数组 × 标的"对，`col = param_idx × n_symbols + symbol_idx`（参数重复、标的平铺）。 | [列轴与列主序](../concepts/column-axis.md) |
| 列主序 | `(row, col)` 存在 `data[col * n_rows + row]`，一列是连续切片，所以"一列"是并行与缓存的工作单位。 | [列轴与列主序](../concepts/column-axis.md) |
| `Columns` / `Level` | `Columns` 按层级 `Level`（如 `window`、`symbol`）保存每一列代表什么的标签，不复制矩阵数据。 | [列标签：Columns、Level 与 group_indices](../concepts/column-labels.md) |
| `FlexArray` | 不物化的参数广播：标量、按列、按行、列映射、全矩阵五种变体共用一个 `get(row, col)`。 | [FlexArray 广播与列映射](../concepts/flex-array.md) |
| `column_map` | 逻辑列 → 源价格列的映射（`FlexArray::MappedColumns`），参数扫描因此不复制价格。 | [FlexArray 广播与列映射](../concepts/flex-array.md) |

## 撮合

| 术语 | 含义 | 详见 |
|---|---|---|
| 成交 | 一笔已执行的订单，即 `OrderRecords` 里的一行。 | [barq-records：记录与派生视图](../crates/barq-records.md) |
| 拒单 | 无法成交的订单，以 `Rejected` / `Ignored` 状态返回而不是错误；`Err` 只表示不变量被破坏、整个运行停止。 | [错误与拒单](../concepts/errors-vs-rejections.md) |
| `StatusInfo` | 订单被拒绝或忽略的原因，共 14 种，判别值与 vectorbt 一一对应。 | [拒单原因 StatusInfo](../reference/status-info.md) |
| 容差比较 | `is_close` 按 PEP 485 比较（相对 1e-9、绝对 1e-12），但 `inf` 永远不"接近"任何值，因为 `size = +inf`、`price = +inf` 是活的哨兵。 | [浮点容差与零钉](../concepts/float-tolerance.md) |
| 零钉 | 余额更新用的 `add(a, b)` 在两数几乎相消时返回精确的 `0.0`，只用于现金、持仓、负债、可用现金这些应该能归零的余额。 | [浮点容差与零钉](../concepts/float-tolerance.md) |
| 换算链 | 内核把 `size_type` 逐级改写成更低级的类型（`TargetPercent` → `TargetValue` → `TargetAmount` → `Amount`），最终只有 `Amount` 到达买卖分支。 | [数量换算链与 percent](../algorithms/size-cascade.md) |
| `percent` | 可用资源的比例而不是目标仓位：只有 `Percent` 类型会设置它，正数在 `buy` 里乘现金预算，负数在 `sell` 里乘可卖数量。 | [数量换算链与 percent](../algorithms/size-cascade.md) |
| `debt` | 每列空头的负债，开空时增加 `short_size × 价格`，平空时按开空均价 `debt / abs(position)` 释放。 | [空头保证金：debt 与 free_cash](../concepts/short-margin.md) |
| `free_cash` | 扣掉空头预留后的可用现金，开空时变化 `proceeds − 2·short_value`、平空时对称释放，共享账户里整组一份。 | [空头保证金：debt 与 free_cash](../concepts/short-margin.md) |
| `lock_cash` | 让后续订单遵守已记下的空头预留：打开时买单的预算从 `free_cash` 起算而不是从 `cash` 起算。 | [buy：买入、平空与 lock_cash 预算](../algorithms/buy-leg.md) |

## 模拟

| 术语 | 含义 | 详见 |
|---|---|---|
| 账户 / 组 | 共享现金的相邻列（`group_lens` 给出每组列数），不分组即每列一个账户，账户组也是并行的单位。 | [barq-sim：模拟循环](../crates/barq-sim.md) |
| 调用顺序（`call_seq`） | 共享账户内各列在一根 K 线里的执行顺序，`Auto` 按估算的订单金额升序排，让卖单先释放现金。 | [call_seq=auto：共享账户内的执行顺序](../algorithms/call-seq-auto.md) |
| 止损锚 | 止损价位的基准（`level = anchor × (1 ∓ stop)`），新开仓或反向时按 `stop_entry_price`（`ValPrice` / `Price` / `FillPrice` / `Close`）重设。 | [止损引擎：触发、跟踪与重新布防](../algorithms/stop-engine.md) |
| trail | `sl_trail` 打开时止损锚跟着持仓期间的极值走（多头取 high、空头取 low），并且在本根止损测试之后才更新。 | [止损引擎：触发、跟踪与重新布防](../algorithms/stop-engine.md) |
| 冲突解析 | `from_signals` 用四个纯函数按固定顺序处理止损出场、同向进出、多空进场和反向进场，顺序一换，幸存的信号就不同。 | [信号冲突解析链与 signals_to_size](../algorithms/signal-conflicts.md) |
| 现金流记录（`CashRecords`） | 入金、收支、分红这类不是成交的现金变动，作为 `OrderRecords.cash` 落成记录，由分析层照样重建。 | [成交之外的现金流](../concepts/cash-flows.md) |
| 无状态随机数 | 随机数是 `(seed, row, col)` 经 SplitMix64 混合得到的函数，不依赖线程数、分组或之前抽过几次。 | [无状态随机数 SplitMix64](../concepts/stateless-rng.md) |
| 阶段 1 / 阶段 2 | 向量化层（已有）/ 带 NautilusTrader 兼容 API 的事件驱动层（计划中）。 | [路线图与已知缺口](../guides/roadmap.md) |

## 记录与分析

| 术语 | 含义 | 详见 |
|---|---|---|
| 记录 | 结构体数组（SoA）形式的成交（`OrderRecords`，含 `CashRecords`）和日志（`LogRecords`），是分析层唯一的数据来源。 | [barq-records：记录与派生视图](../crates/barq-records.md) |
| 日志行 | 一笔被考虑的订单，含执行前后的状态、请求、结果和 `order_id`（没有成交时为 −1）。 | [barq-records：记录与派生视图](../crates/barq-records.md) |
| 三种交易视图 | 同一组成交的三种口径：`entry_trades` 一笔进场一条、`exit_trades` 一笔出场一条、`positions` 一个仓位一条，三者只在 Σ pnl 上一致。 | [三种交易视图](../algorithms/trade-views.md) |
| 回撤段 | 净值从峰值跌下，到回到峰值（`Recovered`）或到最后一根仍在水下（`Active`）为止的一段，深度是 `(valley − peak)/peak`。 | [回撤段识别](../algorithms/drawdowns.md) |
| MAE / MFE | 每笔交易持仓期间相对进场价的最大不利 / 最大有利偏移，`edge_ratio = mean(mfe_ret) / mean(abs(mae_ret))`。 | [MAE / MFE 与 edge ratio](../algorithms/excursions.md) |
| 重建链 | `asset_flow → assets → cash_flow → cash → value → returns`：分析层只从记录重建余额、净值和收益。 | [从记录重建净值](../algorithms/rebuild-chain.md) |
| PSR | 概率夏普比率：考虑样本长度、偏度和峰度之后，真实 Sharpe 高于基准 `SR*` 的概率。 | [过拟合检验：PSR、DSR、PBO](../algorithms/overfitting-stats.md) |
| DSR | Deflated Sharpe Ratio：以"N 次真实 Sharpe 为 0 的试验仅凭运气会看到的期望最大 Sharpe"为基准的 PSR。 | [过拟合检验：PSR、DSR、PBO](../algorithms/overfitting-stats.md) |
| PBO | 回测过拟合概率：样本内最优的列在样本外排名落到中位数或以下（λ ≤ 0）的划分所占的比例。 | [过拟合检验：PSR、DSR、PBO](../algorithms/overfitting-stats.md) |
| CSCV | 组合对称交叉验证：把行切成 S 个块，枚举所有 C(S, S/2) 种"取一半块做样本内"的划分，PBO 在这些划分上计算。 | [过拟合检验：PSR、DSR、PBO](../algorithms/overfitting-stats.md) |
| +inf 陷阱 | 没有交易的列 Sharpe 是 `+inf`，会在 `argmax` 和组均值里登顶，要先用 `traded_only` 把这些列置成 NaN。 | [扫描结果汇总与 +inf 陷阱](../concepts/sweep-aggregation.md) |
| 事件研究 | 记录每个事件前后 `close[t+k]/close[t] − 1` 的收益路径并按偏移聚合，它读未来数据，只能用于分析。 | [事件研究](../algorithms/event-study.md) |

## 因子

| 术语 | 含义 | 详见 |
|---|---|---|
| 因子 | 产生"时间 × 标的"矩阵的具名表达式。 | [barq-expr：因子表达式引擎](../crates/barq-expr.md) |
| Pratt 解析器 | 每个二元算子带一对绑定力 `(left_bp, right_bp)`，遇到 `left_bp < min_bp` 就停，以此处理优先级和右结合的 `**`。 | [Pratt 解析器](../algorithms/pratt-parser.md) |
| 节点 / 槽位 | 依赖图中去重后的运算 / 可复用的暂存序列，节点的最后一个读者算完后槽位就还回空闲栈。 | [依赖图编译：哈希合并、Ref 视图与槽位调度](../algorithms/factor-graph.md) |
| 哈希合并 | 以 `(Op, 输入节点 id)` 为键 intern 节点，结构相同的子表达式就是同一个节点，只算一次。 | [依赖图编译：哈希合并、Ref 视图与槽位调度](../algorithms/factor-graph.md) |
| 视图（`Ref`） | `Ref(x, k)`（`k ≥ 0`）不计算也不占槽位，只记下"底座节点平移 k 行"，读时平移，视图还可以复合。 | [依赖图编译：哈希合并、Ref 视图与槽位调度](../algorithms/factor-graph.md) |
| 滚动累加器 | O(1)/行的窗口统计：对平移后的值做加减、每 `window` 行重建一次限住漂移，常量窗口给出精确的 0。 | [O(1) 滚动累加器](../algorithms/rolling-accumulators.md) |
| 单调队列 | 只保存"仍可能是答案"的下标的双端队列，用来求窗口极值和极值位置，均摊 O(1)。 | [单调队列：窗口极值与极值位置](../algorithms/monotonic-queue.md) |
| 有序环 | 窗口 ≤ 256 时维护的升序 `Vec<f64>`，用 `partition_point` 找位置后插入、删除，服务分位数、排名和 MAD。 | [顺序统计量：有序环与树状数组](../algorithms/order-statistics.md) |
| 树状数组 | 窗口 > 256 或扩展窗口时用的两棵 Fenwick 树（计数与平移和），用二进制提升在 O(log n) 内求第 k 小。 | [顺序统计量：有序环与树状数组](../algorithms/order-statistics.md) |
| Alpha158 / Alpha360 | Qlib 的两套公开特征集，编译后分别是 357 个节点、17 个槽位和 722 个节点、1 个槽位。 | [Alpha158 与 Alpha360](../concepts/alpha-feature-sets.md) |

## 数据与 Python

| 术语 | 含义 | 详见 |
|---|---|---|
| `available_time` | 一条数据实际能被使用的时刻（区别于事件发生的 `event_time`），面板的行轴和对齐都按它。 | [按时点对齐 align](../algorithms/pit-alignment.md) |
| as-of 对齐 | 每个面板时刻 `t` 取最后一个 `available_time ≤ t` 的值；窗口聚合则用左开右闭区间 `(t − width, t]`。 | [按时点对齐 align](../algorithms/pit-alignment.md) |
| F 序 | NumPy 里的列主序（Fortran 序），二维数组进 Rust 必须是 F 序，C 序会被拒绝而不是被转置。 | [Python 边界：F 序与一维的三种含义](../concepts/python-boundary.md) |
| 跨调用缓存 | `PyPortfolio` 拥有记录，并把派生序列存进出借给核心 `Portfolio` 的 `Cache`，一次调用算出的净值曲线供下一次用。 | [PyPortfolio 的跨调用缓存与 GIL](../concepts/portfolio-cache.md) |

## 研究与验证

| 术语 | 含义 | 详见 |
|---|---|---|
| 前视偏差 | 在 `t` 收盘才知道的值在 `t` 就拿来交易；信号工具因此默认延迟一根 K 线，或把延迟当参数。 | [延迟与前视偏差](../concepts/look-ahead.md) |
| 预注册 | 先把假设和规格写进账本并审批（`registered`），再一次性运行，失败的运行也计入试验数 N。 | [研究协议 barq_lab](../tooling/research-protocol.md) |
| 哈希链账本 | `ledger.jsonl` 的每条记录带上一条的 hash，打开时逐行校验，被篡改就报 `LedgerError`。 | [哈希链账本](../algorithms/hash-chain-ledger.md) |
| 合同检查 | `contract_check` 在合成面板上比较前缀计算与全长计算的前段，加了后面的 K 线会改变前面的值就是前视。 | [评估框架与合同检查](../algorithms/research-harness.md) |
| Bonferroni 门槛 | t 统计量门槛随试验数 N 上调，双侧为 `Φ⁻¹(1 − α/(2N))`。 | [门槛、多重比较与原创性检查](../algorithms/research-gates.md) |
| purge | 从训练集去掉测试段之前 `purge` 行，因为这些行的标签向前看会碰到测试段。 | [时间序列交叉验证](../algorithms/ts-cross-validation.md) |
| embargo | 从训练集去掉测试段之后 `embargo` 行，因为这些行的特征向后看会碰到测试段。 | [时间序列交叉验证](../algorithms/ts-cross-validation.md) |
| walk-forward | 每个切分在训练集上选参数，只记录所选参数的测试分数，其余测试分数丢弃。 | [时间序列交叉验证](../algorithms/ts-cross-validation.md) |
| 三重障碍 | 从 `t+1` 往后走，先碰上障碍标 +1、先碰下障碍标 −1、到最长持有期都没碰标 0，并返回决定标签的那一行。 | [机器学习标签](../algorithms/ml-labels.md) |
| fixture | 参考实现（vectorbt 1.1.0 / Qlib）在外部环境生成并提交进仓库的 TSV 期望输出，`cargo test` 不需要 Python 就能重放。 | [差分测试与 fixture](../tooling/parity-testing.md) |
| 变异测试 | 手工给代码施加变异，看测试能否抓住它，抓不住的"幸存者"要论证不可达或等价。 | [差分测试与 fixture](../tooling/parity-testing.md) |

## 相关

- [贯穿全仓库的约定](../guides/conventions.md)：列轴、列主序、容差、拒单、记录是唯一来源、线程不变性这些术语背后的七条约定。
- [系统全景](../guides/system-map.md)：术语分组对应的各层在流水线里的位置。
- [核心算法一览](../guides/algorithm-index.md)：多数术语对应表里的一个算法或机制。
- [延伸资料](../reference/further-reading.md)：PSR、CSCV、hash-consing、purge 等术语的原始出处。
