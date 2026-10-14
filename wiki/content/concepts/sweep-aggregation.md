---
title: 扫描结果汇总与 +inf 陷阱
kind: concept
summary: group_by 按列标签归约扫描结果、best_column 选出赢家；零交易列的 Sharpe 是 +inf，要先用 traded_only 置 NaN。
parent: phasor-analysis
related: [column-labels, metrics, stats-registry, strategy-sweep, indicator-sweep, overfitting-stats]
tags: [analysis, sweep, aggregation]
sources:
  - crates/phasor-analysis/src/aggregate.rs
  - crates/phasor-analysis/src/portfolio.rs:919
  - crates/phasor/tests/sweep.rs
order: 1103
---

# 扫描结果汇总与 +inf 陷阱

参数扫描的结果是一列一个数（每组参数一列），`aggregate.rs` 把它们按列标签归约或选出最好的一列；这里最容易出错的不是归约本身，而是没有交易的列。

## 位置

- `crates/phasor-analysis/src/aggregate.rs`：16–37 行的 +inf 陷阱说明；`group_by`、`best_column`
- `crates/phasor-analysis/src/portfolio.rs:919`：`Portfolio::traded_only`
- `crates/phasor/tests/sweep.rs`：同时钉住朴素排名找到未交易列、过滤后找到真实列这两半

## 汇总

`group_by(columns, level, values, Reduce)` 用[列标签](../concepts/column-labels.md)的 `group_indices` 按标签归约；`best_column` 用 `argmax` 并用 `columns.describe(col)` 命名赢家。

## +inf 陷阱

陷阱（16–37）：没交易的列收益 0、波动 0 → Sharpe `+inf`，`argmax` 和 `Reduce::Mean` 都不跳过无穷，于是它登顶并把组均值变成无穷。解法是先 `Portfolio::traded_only(&values)`，把零交易的列置 NaN（所有归约都跳过 NaN）。`sortino`、`omega`、`tail_ratio`、`information_ratio`、`capture`、`profit_factor` 都有同样的 +inf 出口。

## 相关

- [phasor-analysis：分析层](../crates/phasor-analysis.md)：所属 crate；调用链最后一行就是 `traded_only → group_by / best_column`。
- [列标签：Columns、Level 与 group_indices](../concepts/column-labels.md)：`group_by` 按哪一层标签分组、`describe` 怎样给赢家命名。
- [指标公式表](../reference/metrics.md)：每个 `+inf` 出口的边界条件。
- [@phasor.strategy 的参数扫描](../algorithms/strategy-sweep.md)：Python 侧产生这些扫描列的地方。
- [指标参数网格与共享缓存](../algorithms/indicator-sweep.md)：列标签的层级从参数网格来。
- [过拟合检验：PSR、DSR、PBO](../algorithms/overfitting-stats.md)：选出赢家之后，问它有多少是运气。
