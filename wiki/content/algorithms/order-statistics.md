---
title: 顺序统计量：有序环与树状数组
kind: algorithm
summary: Quantile、Med、Rank、Mad 需要有序窗口：窗口 ≤ 256 用有序环，更长或扩展窗口用按排序位置建的两棵树状数组，两条路径逐位一致。
parent: barq-expr
related: [rolling-accumulators, monotonic-queue, native-evaluation, alpha-feature-sets]
tags: [rolling, data-structures, fenwick]
sources:
  - crates/barq-expr/src/order.rs
order: 706
---

# 顺序统计量：有序环与树状数组

分位数、中位数、时序排名和平均绝对偏差都要求窗口有序，而有序性无法像和那样增量加减。`order.rs` 按窗口长度在两种数据结构之间选择，二者给出逐位相同的结果。

## 位置

- `crates/barq-expr/src/order.rs:1-28`：模块文档
- `crates/barq-expr/src/order.rs:40-62`：`ordered_into`（按窗口长度二选一）
- `crates/barq-expr/src/order.rs:69-118`：有序环 `ring`
- `crates/barq-expr/src/order.rs:120-241`：树状数组 `Ordered`
- `crates/barq-expr/src/order.rs:307`：测试 `ring_and_tree_agree_bit_for_bit`

## 两条路径

`Quantile`、`Med`、`Rank`、`Mad` 都要窗口有序。每行重排是 O(w log w)，这里按窗口长度二选一：

**有序环**（窗口 ≤ 256，`ring` 69–118）：一个升序 `Vec<f64>`；离开值用 `partition_point(total_cmp)` 找到并 `remove`，进入值同样找位 `insert`。O(log w) 查找 + O(w) 搬移，256 个 f64 是 2 KB、几条缓存线。Alpha158 的所有窗口 ≤ 60，所以永远走这条路。

**树状数组**（窗口 > 256 或扩展窗口，`Ordered` 120–241）：整列的非 NaN 值排序一次（并列按行号打破，得到全序），每行一个固定的 `position`；两棵 1-based BIT：`counts`（窗口成员资格）和 `sums`（`value − shift`）。

## 树状数组上的操作

```text
update(row, insert)：rank = position[row]；沿 i += i & −i 更新 counts[i] ±1、sums[i] ±d       O(log n)
kth(k)：二进制提升——at = 0，remaining = k + 1，step = 最高的 2 的幂；
        step 逐次减半，若 counts[at + step] < remaining 则 at += step，remaining −= counts[at]；返回 sorted[at]   O(log n)
prefix(end)：标准 BIT 前缀查询，返回 (count, 平移和)
quantile(q)：index = (count − 1)·q；kth(floor) 与 kth(ceil) 线性插值（NumPy 默认，Hyndman–Fan type 7）
rank(last)：below = prefix(#{v < last}).count；not_above = prefix(#{v ≤ last}).count；
            (below + (equal + 1)/2) / count        ← scipy percentileofscore(kind='rank')：当前行在自己尾窗里的百分位
mad()：kth(0) == kth(count−1) → 0；mean = total/n；用 prefix 在 mean 处切开，Σ|v − mean| 由两侧的和与计数闭式算出   O(log n)
```

## 两条路径的一致性

两条路径是同一定义的两种实现，`ring_and_tree_agree_bit_for_bit` 在含 NaN、`0.0`、`−0.0` 的 700 行序列上对窗口 `{1,2,3,30,60,255,256}` 断言逐位相等。

## 相关

- [barq-expr：因子表达式引擎](../crates/barq-expr.md)：所属 crate；阅读陷阱第 6、7 条（`Rank` 是时序百分位、`Mad` 关于均值）说的就是这里的语义。
- [O(1) 滚动累加器](../algorithms/rolling-accumulators.md)：不需要有序窗口的统计量走的路径。
- [单调队列：窗口极值与极值位置](../algorithms/monotonic-queue.md)：只需要极值时的更便宜的结构，同在 `order.rs` 的 `IdxMax`/`IdxMin`。
- [原生求值：按标的并行走图](../algorithms/native-evaluation.md)：`rolling_into` 把这四个算子分派到 `ordered_into`；树状数组路径是每列 O(n log n) 的例外。
- [Alpha158 与 Alpha360](../concepts/alpha-feature-sets.md)：所有窗口 ≤ 60，所以永远走有序环。
