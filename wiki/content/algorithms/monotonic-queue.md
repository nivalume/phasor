---
title: 单调队列：窗口极值与极值位置
kind: algorithm
summary: 队列里只留"仍可能是答案"的下标，每个下标入队出队各一次，滚动 Max/Min 与 Qlib 语义的 IdxMax/IdxMin 都是均摊 O(1)。
parent: barq-expr
related: [rolling-accumulators, order-statistics, generic-windows, alpha-feature-sets]
tags: [rolling, data-structures, deque]
sources:
  - crates/barq-generic/src/rolling.rs:208-258
  - crates/barq-expr/src/order.rs:246-298
order: 705
---

# 单调队列：窗口极值与极值位置

最大值没有"减法"：值离开窗口时无法从一个运行中的最大值里把它减掉，所以[累加器](../algorithms/rolling-accumulators.md)那套 push/pop 不适用。单调队列换了个思路：一个被后来者支配的下标永远不会再是答案，可以立即丢掉。

## 位置

- `crates/barq-generic/src/rolling.rs:208-257`：`rolling_min_1d`/`rolling_max_1d` 与它们共用的 `rolling_extremum_1d`（`Max`/`Min`）
- `crates/barq-expr/src/order.rs:246-298`：`rolling_arg_extreme_into`（`IdxMax`/`IdxMin`）
- `crates/barq-expr/src/eval.rs:556-588`：`rolling_into` 把 `Max`/`Min` 转给 `barq_generic`

## Max / Min

**Max/Min**（`barq-generic/src/rolling.rs:208-257`）：`candidates: VecDeque<usize>` 存"仍可能是答案"的下标，每个都严格优于其后所有元素。进入新值时从队尾弹出所有不优于它的；离开窗口的下标如果在队首就弹掉。输出 `a[front]`。每个下标入队出队各一次 → 均摊 O(1)；是比较不是累加，所以与重扫逐位相同。

## IdxMax / IdxMin

**IdxMax/IdxMin**（`order.rs:246-298`）：Qlib 用 `np.argmax`，语义是**窗口里有 NaN 就返回第一个 NaN 的位置**，否则第一个极值的位置，1-based。实现：一个 FIFO 的 `nans` 队列 + 一个单调队列（比较用严格 `<`，所以平局保留最早的），输出 `(best − start + 1)`，`best = nans.front() 或 candidates.front()`。Alpha158 的 `IMAX = IdxMax($high, d)/d`。

## 同一个实现：rolling_extremum_1d

barq-expr 没有自己的 `Max`/`Min` 核：`rolling_into` 以 `min_periods = Some(1)` 调用 [barq-generic](../crates/barq-generic.md) 的 `rolling_max_1d`/`rolling_min_1d`，两者都是私有函数 `rolling_extremum_1d`（`crates/barq-generic/src/rolling.rs:221-258`）的薄包装，上面描述的正是它；同一个函数还承担 barq-generic 自己的矩阵版 `rolling_min`/`rolling_max`（与 vectorbt 对照）和 [barq-indicators](../crates/barq-indicators.md) 里 `stoch` 的区间高低点，它只在队列之外多数一个非 NaN 计数 `cnt` 来判断 `min_periods`（模块文档 `rolling.rs:22-26` 写了为什么极值用队列而均值用累加）。`rolling_arg_extreme_into` 是同一结构的变体：多一个 `nans` 队列，并把弹出条件从"不优于"（求最大时 `a[j] <= v`）收紧成严格的 `<`，所以平局时队首留下的是最早的下标而不是最晚的。

## 相关

- [barq-expr：因子表达式引擎](../crates/barq-expr.md)：所属 crate；阅读陷阱第 7 条（`IdxMax` 遇到 NaN 就指向 NaN）出自这里。
- [barq-generic：通用窗口算子](../crates/barq-generic.md)：`rolling_extremum_1d` 所在的 crate，Max/Min 的实际实现。
- [O(1) 滚动累加器](../algorithms/rolling-accumulators.md)：能做减法的统计量走的另一条路。
- [顺序统计量：有序环与树状数组](../algorithms/order-statistics.md)：需要整个有序窗口而不只是极值时的结构。
- [前缀和与 EWM 窗口核](../algorithms/generic-windows.md)：barq-generic 里与极值并列的其他窗口核。
- [Alpha158 与 Alpha360](../concepts/alpha-feature-sets.md)：`IMAX`/`IMIN`、`MAX`/`MIN` 族用到这里的两个核。
