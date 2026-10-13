---
title: 合并每线程缓冲与 order_id 重基
kind: algorithm
summary: OrderRecords::concat 按块顺序拼接每线程缓冲，大时用 split_at_mut 并行拷贝；Journal::extend 是唯一给日志 order_id 加偏移的地方。
parent: barq-records
related: [parallel-simulation, colmap, barq-sim, rebuild-chain]
tags: [records, merge, parallel]
sources:
  - crates/barq-records/src/lib.rs:252-305
  - crates/barq-records/src/logs.rs:464-484
order: 901
---

# 合并每线程缓冲与 order_id 重基

[按账户组并行](../algorithms/parallel-simulation.md)时，每个线程把成交写进自己的缓冲，`S::merge(parts)` 再按块顺序把它们拼成一个。成交部分只是拷贝；麻烦在日志：每条日志用 `order_id` 指向它产生的成交，而这个编号是相对于写入它的那个缓冲的。

## 位置

- `crates/barq-records/src/lib.rs:252-305`（`OrderRecords::concat`）
- `crates/barq-records/src/logs.rs:464-484`（`Journal::extend`、`Journal::concat`）

## 算法

```text
OrderRecords::concat(parts):
    total < 65,536 或 parts < 2 → 逐个 extend（含各自的 cash）
    否则：六个 Vec 用零填充到 total（calloc 惰性映射，首次写入的线程拥有物理页）
          cash 串行拼接；std::thread::scope 里每个 part 一个线程，
          split_at_mut 切出互不重叠的区间，copy_from_slice —— 借用检查器证明不相交，没有 unsafe
Journal::extend(other):
    offset = self.orders.len()   ← 追加之前
    追加 orders 与 logs；本次追加的日志里 order_id ≥ 0 的加 offset，−1 不动
```

## 为什么必须重基

每个线程的缓冲从 0 开始编号成交；不重基，日志会指向一个"看起来合理但错误"的成交。只有 `Journal` 知道偏移，`LogRecords::concat` 自己**不**重基。

## 相关

- [barq-records：记录与派生视图](../crates/barq-records.md)：所属 crate；`SinkBuffer::merge` 与"id 就是下标"的契约。
- [按账户组并行与确定性合并](../algorithms/parallel-simulation.md)：调用这里的并行驱动，以及合并后逐位一致的理由。
- [ColMap 稳定计数排序与 MappedArray](../algorithms/colmap.md)：合并过的记录不再连续时走的慢路径。
- [barq-sim：模拟循环](../crates/barq-sim.md)：共享组记录是执行顺序、排序后 `Journal.order_id` 指向排序前下标的陷阱。
