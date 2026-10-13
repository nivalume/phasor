---
title: 指标公式表
kind: reference
summary: barq-analysis 的收益指标、基准指标与交易统计的公式和边界情况，以及滚动版本为什么朴素地逐窗重算。
related: [barq-analysis, stats-registry, sweep-aggregation, rebuild-chain, overfitting-stats]
tags: [metrics, analysis, formulas]
sources:
  - crates/barq-analysis/src/metrics.rs
  - crates/barq-analysis/src/benchmark.rs
  - crates/barq-analysis/src/trade_stats.rs
  - crates/barq-analysis/src/percentile.rs
  - crates/barq-analysis/src/rolling.rs
order: 20
---

# 指标公式表

[barq-analysis](../crates/barq-analysis.md) 在[重建链](../algorithms/rebuild-chain.md)给出的收益列上计算的每个指标：公式按参考实现（vectorbt 的 `returns/nb.py`，血统是 empyrical）的拼写，边界一列写出 NaN、`+inf` 从哪里来。

## 位置

- `crates/barq-analysis/src/metrics.rs`、`crates/barq-analysis/src/percentile.rs`（`percentile` 43–79）
- `crates/barq-analysis/src/benchmark.rs`
- `crates/barq-analysis/src/trade_stats.rs`（51–98）
- `crates/barq-analysis/src/rolling.rs`；`rolling_apply_1d` 在 `crates/barq-generic/src/rolling.rs:71-99`

## 记号

记 `r` 为收益列、`n` 为非 NaN 根数、`A = ann_factor`（默认 365）、`rf = risk_free`（每根）。

## 公式与边界

| 指标 | 公式 | 边界 |
|---|---|---|
| `nanstd(r, ddof)` | `sqrt(Σ(x−mean)²/n × n/(n−ddof))` 先总体再缩放（参考实现的拼写） | `n ≤ ddof` → NaN |
| `total_return` | `Π(1+r) − 1` | |
| `annualized_return` | `(Π(1+r))^(A / len) − 1`，**`len` 含 NaN 根** | 空 → NaN |
| `annualized_volatility` | `nanstd(r,1) × √A` | `len < 2` → NaN |
| `max_drawdown` | `cum = Π(1+r)`，`level = 100 × cum`，`dd = level/runmax − 1` 的**NaN 传播**最小值 | 唯一传播 NaN 的指标：净值到无穷的列不能答 0 |
| `sharpe_ratio` | `nanmean(r − rf) / nanstd(r − rf, 1) × √A`（不分配，`nanmean_nanstd_shifted`） | `std == 0` → **+inf** |
| `downside_risk` | `sqrt(Σ min(r − req, 0)² / count) × √A` | |
| `sortino_ratio` | `nanmean(r − req) × A / downside_risk` | `risk == 0` → +inf；注册表传 `req = 0` |
| `calmar_ratio` | `annualized_return / \|max_drawdown\|` | `mdd == 0` → NaN |
| `omega_ratio` | `Σ(r − rf − thr)⁺ / Σ(r − rf − thr)⁻`，`thr = (1+req)^(1/A) − 1` | 分母 0 → +inf |
| `tail_ratio` | `\|P95\| / \|P5\|` | |
| `value_at_risk` | 去 NaN 后 `percentile(5%)` | |
| `cond_value_at_risk` | **不去 NaN**，`cutoff = floor((len−1) × 0.05)`，NaN 排最后，取前 `cutoff+1` 个的均值 | 尾部有 NaN → NaN |
| `percentile(v, q)` | `rank = 1 + (n−1)q/100`，`lower × (1−m) + upper × m`——NumPy 默认线性插值（Hyndman–Fan type 7）的 Numba 拼写 | 任一 NaN → NaN |
| `information_ratio` | `nanmean(r − b) / nanstd(r − b, 1)` | std 0 → +inf |
| `beta` | `b` 在 `r` 为 NaN 处也置 NaN；`cov = nanmean((b − mean b) × r)`，`var = nanmean((b − mean b)²)` | `var < 1e-30` → NaN |
| `alpha` | `(nanmean((r − rf) − β(b − rf)) + 1)^A − 1` | |
| `up/down_capture` | 只取 `b > 0` / `b < 0` 的根，年化收益之比 | |
| `TradeStats` | `wins = pnl > 0`、`losses = pnl < 0`（平手两边都不算）；`profit_factor = Σwins/\|Σlosses\|`；`expectancy = win_rate × avg_win − (1 − win_rate) × \|avg_loss\|`；**`sqn = √count × mean(pnl) / std(pnl, 1)`** | 无亏损 → +inf |

表里的每个 `+inf` 出口在扫描汇总时都会登顶排名，处理办法见[扫描结果汇总与 +inf 陷阱](../concepts/sweep-aggregation.md)。

## 滚动指标

`rolling.rs` 的每个滚动指标都是标量版在每个尾窗上**朴素**地跑（`rolling_apply_1d`，O(n·w)，百分位类 O(n·w·log w)），因为百分位、回撤、回归需要整个窗口在手；最后一个完整窗口与标量版逐位相等。

## 相关

- [barq-analysis：分析层](../crates/barq-analysis.md)：这些公式所在的 crate，以及它们与 vectorbt 的逐项对应。
- [统计表注册表](../concepts/stats-registry.md)：`Portfolio.stats()` 的 28 行调用的就是这些函数。
- [扫描结果汇总与 +inf 陷阱](../concepts/sweep-aggregation.md)：`sharpe_ratio` 等指标的 `+inf` 在参数扫描里的后果。
- [过拟合检验：PSR、DSR、PBO](../algorithms/overfitting-stats.md)：那里的 Sharpe 是每根 K 线、不年化、无无风险利率的版本。
- [从记录重建净值](../algorithms/rebuild-chain.md)：收益列 `r` 从哪里来。
- [barq-generic：通用窗口算子](../crates/barq-generic.md)：`rolling_apply_1d` 所在的 crate。
