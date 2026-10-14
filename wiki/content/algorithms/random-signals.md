---
title: 带种子的随机信号
kind: algorithm
summary: 每个抽样只由 (seed, row, col) 决定；random_n 用部分 Fisher–Yates，恰好 n 个、无放回，且 random_n(n) 是 random_n(m) 的前缀。
parent: phasor-signals
related: [stateless-rng, synthetic-prices, signal-generation, research-gates]
tags: [random, signals, seed]
sources:
  - crates/phasor-signals/src/random.rs
order: 403
---

# 带种子的随机信号

`random.rs` 是零模型：进场规则若打不过同样数量的 `random_n` 进场，它就只是一个样本，不是规则（模块文档 12–16 行）。每个抽样都是 `(seed, row, col)` 的纯函数，所以一列的信号不管单独生成还是作为扫描的第一千列都一样，与线程数无关——这是 [无状态随机数 SplitMix64](../concepts/stateless-rng.md) 的同一条约定。

## 位置

- `crates/phasor-signals/src/random.rs:20-35`（`mix`、`uniform`，落在 `[0,1)`）
- `crates/phasor-signals/src/random.rs:48-56`（`pick_rows`：部分 Fisher–Yates）
- `crates/phasor-signals/src/random.rs:66`（`random_by_prob`）
- `crates/phasor-signals/src/random.rs:87`（`random_n`）
- `crates/phasor-signals/src/random.rs:114`（`random_entries_exits`）

## 三个生成器

**`random.rs`**：`random_by_prob` 每格 `uniform < prob`（每列二项分布）；`random_n` 用**部分 Fisher–Yates**（不是拒绝采样）：第 `k` 步的抽样键是 `(seed, k, col)`，`O(n_rows)`，恰好 `n` 个、无放回、列间独立，而且 `random_n(n)` 是 `random_n(m)` 的前缀；`random_entries_exits` 取 `2n` 个不同行排序后交替分配，严格交替、每个进场配一个出场。

随机数不是 NumPy 的流，无法复现 vectorbt 的随机信号；这里的 `uniform` 不带 salt、落在 `[0,1)`，[合成价格](../algorithms/synthetic-prices.md)的那份带 salt、落在 `(0,1)`。

## 相关

- [phasor-signals：信号工具箱](../crates/phasor-signals.md)：所属 crate；三份 SplitMix64 副本待合并（P3）是它的阅读陷阱之一。
- [无状态随机数 SplitMix64](../concepts/stateless-rng.md)：模拟器里的同一个思路，以及为什么随机数不能依赖线程或分组。
- [带种子的合成价格](../algorithms/synthetic-prices.md)：`phasor-generic` 里带 salt 的另一份副本。
- [信号清洗与出场生成](../algorithms/signal-generation.md)：`random_entries_exits` 的输出已经严格交替，与 `clean_enex` 的结果形状相同。
- [门槛、多重比较与原创性检查](../algorithms/research-gates.md)：研究协议的负对照（随机因子、置换、泄露）必须被门槛拒绝，与这里的零模型是同一个思路。
