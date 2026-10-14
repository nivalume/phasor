---
title: 统计表注册表
kind: concept
summary: DEFAULT_METRICS 是 28 行 key/title/tags/compute 的静态表，Select 先按键与标签过滤，未入选的指标不计算。
parent: phasor-analysis
related: [metrics, sweep-aggregation, trade-views, parity-testing]
tags: [analysis, stats, registry]
sources:
  - crates/phasor-analysis/src/stats.rs:323-557
  - crates/phasor-analysis/tests/parity_stats.rs
order: 1102
---

# 统计表注册表

`Portfolio::stats(Select)` 的输出不是写死的一串调用，而是一张静态注册表：每行一个指标的键、标题、标签和计算函数，过滤在计算之前发生。

## 位置

- `crates/phasor-analysis/src/stats.rs:45-235`：类型与 `Select`
- `crates/phasor-analysis/src/stats.rs:323-557`：`DEFAULT_METRICS`
- `crates/phasor-analysis/src/stats.rs:266-274`："合格交易"
- `crates/phasor-analysis/src/stats.rs:568-614`：`BENCHMARK_METRICS`
- `crates/phasor-analysis/tests/parity_stats.rs`：逐行钉住顺序与标题

## 注册表、过滤与合格交易

`static DEFAULT_METRICS: &[Metric]`，28 行，每行 `key / title / tags / compute`，顺序和标题是 vectorbt `Portfolio.stats()` 的，由 `tests/parity_stats.rs` 逐行钉住。
`Select` 过滤：`keys ∧ 全部 require 标签 ∧ 无 exclude 标签`，未入选的指标**不计算**。
"合格交易"（266–274）= 本列已平仓的交易，除非 `incl_open`；它作用于第 16–24 行（胜率、最好/最差、平均盈亏、时长、盈亏比、期望），不作用于三个计数和未平仓盈亏。
`Max Drawdown [%]` 被取反成正数（其他所有回撤数字都是负的）。`BENCHMARK_METRICS`（568–614）另成一表：`beta, alpha, information_ratio, up_capture, down_capture`。

每行 `compute` 调用的公式见[指标公式表](../reference/metrics.md)。

## 相关

- [phasor-analysis：分析层](../crates/phasor-analysis.md)：所属 crate；类图里的 `Metric`、`Select`、`StatsInput`、`Value` 就是这张表的类型。
- [指标公式表](../reference/metrics.md)：注册表每一行背后的公式与边界。
- [三种交易视图](../algorithms/trade-views.md)：`Scoring.trades_type` 决定"合格交易"取自哪一种交易视图。
- [扫描结果汇总与 +inf 陷阱](../concepts/sweep-aggregation.md)：从单列统计走到整个参数扫描的汇总。
- [差分测试与 fixture](../tooling/parity-testing.md)：`generate_stats.py` 生成的 fixture 钉住这 28 行。
