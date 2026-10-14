---
title: 按账户组并行与确定性合并
kind: algorithm
summary: 按列数把账户组切块、每块一个作用域线程、按块顺序合并并重基 order_id，所以线程数不改变任何一位结果。
parent: phasor-sim
related: [record-merge, stateless-rng, bar-loop, conventions, benchmarks]
tags: [parallel, determinism, threads]
sources:
  - crates/phasor-sim/src/groups.rs:156-175
  - crates/phasor-sim/src/from_orders.rs:703-757
  - crates/phasor-records/src/lib.rs:252-305
  - crates/phasor-records/src/logs.rs:464-474
  - crates/phasor-sim/examples/scaling.rs
order: 1006
---

# 按账户组并行与确定性合并

并行单位是账户组：组内各列共享现金，必须按[调用顺序](../algorithms/call-seq-auto.md)串行执行；组与组之间互不相干，可以交给不同线程。难点不在切分，而在合并之后的记录与串行运行逐位一致——这是[贯穿全仓库的约定](../guides/conventions.md)之一。

## 位置

- `crates/phasor-sim/src/groups.rs:156-175`（`Groups::chunks`）
- `crates/phasor-sim/src/from_orders.rs:703-757`（`simulate_into_parallel`）
- `crates/phasor-records/src/lib.rs:252-305`（`OrderRecords::concat`）
- `crates/phasor-records/src/logs.rs:464-474`（`Journal::extend`）
- `crates/phasor-sim/examples/scaling.rs`（扩展性测量）

## 切块、执行、合并

![线程](../../../docs/img/zh/threads.svg)

1. **切块**（`Groups::chunks`，156–175）：`total = Σ lens`，第 `t` 个线程的目标是 `total × t / n_threads`，沿组累加到目标为止；最后一个线程扫尾。按**列数**而不是组数均衡，所以一个宽组自成一块：`[6,1,1,1,1,1,1].chunks(2) == [(0,1),(1,7)]`。
2. **`std::thread::scope`**（`from_orders.rs:703-757`）：每块一个线程，各自 `S::with_capacity(fill_bound(块内列))`、各自的 `fills` 计数器，跑 `simulate_group`。作用域线程强制参数是 `Sync`、状态按列本地。没有共享可变状态、没有锁、没有归约。
3. **合并**：结果按块顺序收集，最小编号块的错误胜出；`S::merge(parts)` → `OrderRecords::concat`（超过 65,536 条并行拷贝）或 `Journal::concat` → `extend`：`offset = 已有订单数`，追加日志里每个非负 `order_id += offset`，`−1` 保持。
4. **为什么逐位一致**：记录按 `(col, idx)` 定位、id 是合并后的位置、随机数只依赖 `(seed, row, col)`。测试 `the_parallel_path_is_identical_to_the_serial_one`。

第 2 步里每个线程跑的 `simulate_group` 见[from_orders 的一根 K 线](../algorithms/bar-loop.md)；第 3 步的合并算法见[合并每线程缓冲与 order_id 重基](../algorithms/record-merge.md)；第 4 步的随机数见[无状态随机数 SplitMix64](../concepts/stateless-rng.md)。

## 扩展性

扩展性（`examples/scaling.rs`，4 核）：稀疏负载 3.62×，密集负载 1.63×（内存带宽受限，~245 MB 记录写入）。

## 相关

- [phasor-sim：模拟循环](../crates/phasor-sim.md)：所属 crate；plain / logged / serial / parallel 四种入口。
- [合并每线程缓冲与 order_id 重基](../algorithms/record-merge.md)：`S::merge` 在记录层的实现。
- [无状态随机数 SplitMix64](../concepts/stateless-rng.md)：随机抽样不依赖分组与线程数的原因。
- [from_orders 的一根 K 线](../algorithms/bar-loop.md)：每个线程内部逐组执行的循环。
- [贯穿全仓库的约定](../guides/conventions.md)：串行与并行必须逐位一致的约定。
- [基准协议](../tooling/benchmarks.md)：扩展性数字的测量方法。
