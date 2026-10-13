---
title: 核心算法一览
kind: guide
summary: 读完应该能脱稿讲清楚的 27 个算法与机制，为什么值得单独学，以及讲它的页面。
related: [reading-roadmap, conventions, exercises, glossary]
tags: [algorithms, index]
order: 40
---

# 核心算法一览

读完本知识库你应该能脱稿讲清楚下面每一项。最后一列是讲它的页面。

| 算法 / 机制 | 为什么值得单独学 | 页面 |
|---|---|---|
| 列轴布局与 `FlexArray::MappedColumns` | 参数扫描不复制价格的根本原因 | [列轴与列主序](../concepts/column-axis.md)、[FlexArray 广播与列映射](../concepts/flex-array.md) |
| PEP 485 容差 + `inf` 哨兵 + `add` 零钉 | 变异测试里唯一的幸存者就死在这里 | [浮点容差与零钉](../concepts/float-tolerance.md) |
| 数量换算链（TargetPercent → … → Amount）与买卖分支 | 内核的全部语义；14 种拒单原因各在哪一行 | [数量换算链与 percent](../algorithms/size-cascade.md)、[buy](../algorithms/buy-leg.md)、[sell](../algorithms/sell-leg.md)、[拒单原因 StatusInfo](../reference/status-info.md) |
| 做空的 `debt` / `free_cash` 记账（2 × 空头市值） | 路径相关的状态为什么要显式保存 | [空头保证金：debt 与 free_cash](../concepts/short-margin.md) |
| 费用反解 `(cash_limit − fixed) / (1 + fees)` + 二次粒度取整 | 费用在上限之内而不是之上 | [buy：买入、平空与 lock_cash 预算](../algorithms/buy-leg.md) |
| 一根 K 线两遍扫描：先定价/止损/排序，再执行 | 共享账户的 `call_seq="auto"` 为什么需要两遍 | [from_orders 的一根 K 线](../algorithms/bar-loop.md)、[call_seq=auto：共享账户内的执行顺序](../algorithms/call-seq-auto.md) |
| 止损引擎：跳空按开盘、触及按价位、测完再 trail | 先 trail 后测会在新高当根就被打出来 | [止损引擎：触发、跟踪与重新布防](../algorithms/stop-engine.md) |
| 信号冲突解析链（4 个纯函数的固定顺序） | 顺序一换，幸存的信号就不同 | [信号冲突解析链与 signals_to_size](../algorithms/signal-conflicts.md) |
| `std::thread::scope` 按账户组切块 + 顺序合并 + `order_id` 重基 | 线程数不改变输出 | [按账户组并行与确定性合并](../algorithms/parallel-simulation.md)、[合并每线程缓冲与 order_id 重基](../algorithms/record-merge.md) |
| SplitMix64 对 `(seed,row,col)` 的无状态抽样 | 可复现、可并行的随机拒单 | [无状态随机数 SplitMix64](../concepts/stateless-rng.md) |
| 三种交易视图（entry FIFO / exit 平均 / position 分组） | 同一组成交，三种统计口径 | [三种交易视图](../algorithms/trade-views.md) |
| 稳定计数排序 `ColMap` + `MappedArray` 归约 | 记录按列分组的零拷贝快路径 | [ColMap 稳定计数排序与 MappedArray](../algorithms/colmap.md) |
| 重建链 `asset_flow → assets → cash_flow → cash → value → returns` | 分析只信记录 | [从记录重建净值](../algorithms/rebuild-chain.md) |
| 含入金的收益 `value[t] / (value[t−1] + deposit[t]) − 1` | 入金不是收益，出金不是回撤 | [从记录重建净值](../algorithms/rebuild-chain.md) |
| PSR / 期望最大 Sharpe / DSR / PBO-CSCV | 扫了一千组参数后"最好"的那组有多少是运气 | [过拟合检验：PSR、DSR、PBO](../algorithms/overfitting-stats.md) |
| Pratt 解析器 | 优先级与右结合 `**` 的最小实现 | [Pratt 解析器](../algorithms/pratt-parser.md) |
| 哈希合并（hash-consing）+ 常量折叠 + 最后读者释放槽位 | 357 个节点只用 17 个暂存序列 | [依赖图编译：哈希合并、Ref 视图与槽位调度](../algorithms/factor-graph.md) |
| `Ref` 视图：平移读而不计算 | Alpha360 的 354 个节点一个槽位 | [依赖图编译：哈希合并、Ref 视图与槽位调度](../algorithms/factor-graph.md)、[Alpha158 与 Alpha360](../concepts/alpha-feature-sets.md) |
| 滚动累加器：平移求和、每 N 行重建、常量窗口精确零 | O(1)/行，但要和 Qlib 的 `== 0` 逐位一致 | [O(1) 滚动累加器](../algorithms/rolling-accumulators.md) |
| 单调队列求窗口极值 / 极值位置 | 均摊 O(1) | [单调队列：窗口极值与极值位置](../algorithms/monotonic-queue.md) |
| 有序环 + 树状数组求分位数 / 排名 / MAD | 窗口 ≤ 256 走环，否则 O(log n) 树 | [顺序统计量：有序环与树状数组](../algorithms/order-statistics.md) |
| 三重障碍标签、pivot 标签、固定期收益 | 读未来的函数只能做标签 | [机器学习标签](../algorithms/ml-labels.md) |
| 按 `available_time` 的 as-of / 窗口对齐 | 防止用到晚发布的数据 | [按时点对齐 align](../algorithms/pit-alignment.md) |
| Fortran 序零拷贝进 Rust、GIL 释放、`Cache` 跨调用复用 | Python 边界的三个规则 | [Python 边界：F 序与一维的三种含义](../concepts/python-boundary.md)、[PyPortfolio 的跨调用缓存与 GIL](../concepts/portfolio-cache.md) |
| purged k-fold + embargo、walk-forward 在训练集选参 | 时间序列交叉验证 | [时间序列交叉验证](../algorithms/ts-cross-validation.md) |
| 哈希链账本 + 预注册 + Bonferroni 门槛 | 模型永远看不到结果 | [哈希链账本](../algorithms/hash-chain-ledger.md)、[研究协议 barq_lab](../tooling/research-protocol.md)、[门槛、多重比较与原创性检查](../algorithms/research-gates.md) |
| 差分测试：外部环境生成 TSV fixture，`cargo test` 重放 | 没有 Python 也能验证到参考实现 | [差分测试与 fixture](../tooling/parity-testing.md) |

## 相关

- [阅读路线](../guides/reading-roadmap.md)：按什么顺序读到这些算法。
- [贯穿全仓库的约定](../guides/conventions.md)：几乎每个算法都依赖的 7 个约定。
- [练习与自测](../guides/exercises.md)：用动手练习检验能不能讲清楚。
- [术语表](../reference/glossary.md)：表里术语的含义。
