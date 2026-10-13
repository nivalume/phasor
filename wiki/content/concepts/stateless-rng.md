---
title: 无状态随机数 SplitMix64
kind: concept
summary: 每个随机数都是 (seed, row, col) 的纯函数，所以结果与分组和线程数无关；仓库里有三份实现，映射区间有两种。
parent: barq-sim
related: [parallel-simulation, random-signals, synthetic-prices, roadmap, conventions]
tags: [random, determinism, parallel]
sources:
  - crates/barq-sim/src/random.rs
  - crates/barq-sim/src/from_orders.rs:485-486
  - crates/barq-signals/src/random.rs:20-56
  - crates/barq-generic/src/synthetic.rs:29-71
  - docs/plan.md
order: 1008
---

# 无状态随机数 SplitMix64

`barq-sim` 只在一个地方需要随机数：`reject_prob > 0` 时决定一笔订单是否被随机拒绝（内核自己从不抽样，抽样结果由循环传进去）。它不用一个随循环推进的生成器，而是把每次抽样写成 `(seed, row, col)` 的函数——这是[按账户组并行与确定性合并](../algorithms/parallel-simulation.md)能做到逐位一致的条件之一。

## 位置

- `crates/barq-sim/src/random.rs:10-24`（`mix`、`uniform`）
- `crates/barq-sim/src/from_orders.rs:485-486`、`crates/barq-sim/src/from_signals.rs:866-867`（只在 `reject_prob > 0` 时抽）

## 算法

```text
mix(z) = SplitMix64 输出函数（64 位双射）
uniform(seed, row, col) = mix(mix(mix(seed) ^ row) ^ col) 取高 53 位 / 2^53  → [0, 1) 内精确值
```

共享生成器的第 n 次抽样取决于之前抽了多少次，而那取决于分组和线程数；`(seed, row, col)` 的函数两者都不依赖。只在 `reject_prob > 0` 时才抽（`from_orders.rs:485-486`）。`plan.md` P3 记录了三份相同实现待合并到 `barq-core`。

## 另外两份实现

严格说三份只有 SplitMix64 的混合步骤（同一组常数）完全相同；从 64 位整数映射到浮点数有两种做法，[docs/plan.md](../../../docs/plan.md) P3 的"现状"一栏写明了这一点。

| 位置 | 链式哈希 | 映射 | 区间 | 用在 |
|---|---|---|---|---|
| `crates/barq-sim/src/random.rs:20-24` | `mix(mix(mix(seed) ^ row) ^ col)` | `(bits >> 11) / 2^53` | `[0, 1)` | `reject_prob` 的随机拒单 |
| `crates/barq-signals/src/random.rs:32-35` | 同上 | `(bits >> 11) × 2^-53`，与上一行逐位相同 | `[0, 1)` | [带种子的随机信号](../algorithms/random-signals.md)：`random_by_prob` 逐格比较；`random_n` 与 `random_entries_exits` 用部分 Fisher–Yates 洗牌选行，第 `k` 步按 `(seed, k, col)` 抽样 |
| `crates/barq-generic/src/synthetic.rs:48-62` | `splitmix64(splitmix64(splitmix64(seed ^ salt) ^ row) ^ col)` | `((bits >> 11) + 0.5) × 2^-53` | `(0, 1)` | [带种子的合成价格](../algorithms/synthetic-prices.md)：Box–Muller 正态（`random_walk`、`gbm`）与 `ohlc_from_close` 的上下影线 |

`synthetic.rs` 的两处不同各有理由：

- **salt**：同一个 `(seed, row, col)` 上要抽几个互相独立的数——Box–Muller 的 `u1`、`u2`，OHLC 的 high、low——每个抽样有自己的 salt 常量（`SALT_NORMAL_U1`、`SALT_NORMAL_U2`、`SALT_HIGH`、`SALT_LOW`，31–34 行），先与 `seed` 异或再走同样的链，改一个不会挪动另一个。
- **`+ 0.5`**：把 53 位整数放到桶的中点，结果永远不等于 0 也不等于 1，`normal` 里的 `ln(u1)` 因此有限。同样的位在两种映射下相差 `2^-54`，所以合并时两种映射都要保留，否则合成价格的输出会变。

三份目前都是私有函数（`barq-sim` 的是 `pub(crate)`）。P3 的计划是由 `barq-core` 提供混合步骤和带 salt 的链式哈希、两种均匀映射各自保留，并且先加测试钉住三处现有的输出再改；E4（自定义模拟循环）要公开的"按 `(seed, row, col)` 取随机数的函数"也和 P3 一起做。进度见[路线图与已知缺口](../guides/roadmap.md)。

## 相关

- [barq-sim：模拟循环](../crates/barq-sim.md)：所属 crate；`random.rs` 是 vectorbt 没有的 barq 新增部分。
- [按账户组并行与确定性合并](../algorithms/parallel-simulation.md)：随机数只依赖 `(seed, row, col)` 是逐位一致的三个原因之一。
- [带种子的随机信号](../algorithms/random-signals.md)：`barq-signals` 里的第二份实现。
- [带种子的合成价格](../algorithms/synthetic-prices.md)：`barq-generic` 里带 salt、开区间的第三份实现。
- [路线图与已知缺口](../guides/roadmap.md)：P3 合并三份实现、E4 公开随机数函数。
- [贯穿全仓库的约定](../guides/conventions.md)："随机数是 `(seed, row, col)` 的函数"这条约定。
