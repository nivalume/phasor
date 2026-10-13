---
title: 带种子的合成价格
kind: algorithm
summary: 每个抽样都是 (seed, row, col, salt) 的无状态 SplitMix64 函数；gbm 用精确一步解，ohlc_from_close 在负价格下仍保序。
parent: barq-generic
related: [stateless-rng, random-signals, overfitting-stats, research-gates]
tags: [random, synthetic, gbm, seed]
sources:
  - crates/barq-generic/src/synthetic.rs:1-243
order: 303
---

# 带种子的合成价格

`synthetic.rs` 生成性质已知的价格路径，用来做对照而不是模拟市场：零漂移的 `gbm` 按构造没有优势，在它上面报出漂亮夏普的策略是在度量自己的过拟合；`random_walk` 的步长独立，任何"预测"它的信号都在读噪声（模块文档 1–11 行）。随机数与 [barq-signals](../crates/barq-signals.md) 的 `random.rs`、`barq-sim` 的 `random.rs` 是同一个思路，见 [无状态随机数 SplitMix64](../concepts/stateless-rng.md)。

## 位置

- `crates/barq-generic/src/synthetic.rs:29-70`（四个 salt、`splitmix64`、`bits`、`uniform`、`normal`）
- `crates/barq-generic/src/synthetic.rs:112`（`random_walk`）
- `crates/barq-generic/src/synthetic.rs:149`（`gbm`）
- `crates/barq-generic/src/synthetic.rs:202`（`ohlc_from_close`）

## 抽样与三条路径

**`synthetic.rs`**：无状态 SplitMix64，`bits(seed,row,col,salt)` 链式哈希（`(row,col)` 与 `(col,row)` 不同），四个 salt 让同一格的不同抽样独立；`uniform` 取高 53 位 `+0.5` 后缩放，严格在 `(0,1)` 内（`ln` 有限）；`normal` 用 Box–Muller。`random_walk`：`x_t = x_{t−1} + σ·Z`；`gbm`：精确一步解 `S_t = S_{t−1}·exp((μ − σ²/2)dt + σ√dt·Z)`，不是 Euler；`ohlc_from_close`：`open = 前收`，`high = max(o,c) + |·|·spread·U`，`low` 对称，用绝对值保证负价格下仍 `low ≤ min ≤ max ≤ high`。某一列的结果不随 `n_cols` 变化。

`barq-signals` 的 `uniform` 不带 salt、落在 `[0,1)`，这里的带 salt、落在 `(0,1)`；三份 SplitMix64 副本待合并（`docs/plan.md` P3）。随机数不是 NumPy 的流，所以无法复现 vectorbt 的随机结果，`synthetic.rs` 在参考实现里也没有对应，靠性质测试钉住。

## 相关

- [barq-generic：通用窗口算子](../crates/barq-generic.md)：所属 crate。
- [无状态随机数 SplitMix64](../concepts/stateless-rng.md)：同一个"随机数是 `(seed, row, col)` 的函数"约定在模拟器里的版本，以及 P3 的合并计划。
- [带种子的随机信号](../algorithms/random-signals.md)：`barq-signals` 里的第二份副本，不带 salt 的 `uniform`。
- [过拟合检验：PSR、DSR、PBO](../algorithms/overfitting-stats.md)：零漂移 GBM 是这些统计量之外的另一种对照。
- [门槛、多重比较与原创性检查](../algorithms/research-gates.md)：研究协议里随机基线与负对照的用法。
