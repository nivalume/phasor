---
title: 事件研究
kind: algorithm
summary: forward_paths 在每个事件前后记录相对事件当根收盘的收益路径，再按偏移求均值、中位数与命中率；它读未来，只做分析。
parent: phasor-analysis
related: [look-ahead, ml-labels, metrics, signal-sweep]
tags: [analysis, events]
sources:
  - crates/phasor-analysis/src/events.rs
order: 1105
---

# 事件研究

"信号出现之后价格平均怎么走"是一个事件研究问题：`events.rs` 是一个独立模块，把每个事件对齐到同一条偏移轴上，再按偏移聚合。

## 位置

- `crates/phasor-analysis/src/events.rs`：`forward_paths`、`mean_path`、`median_path`、`hit_rate`；输出类型 `EventPaths`

## 路径与聚合

`forward_paths(close, events, before, after)`：对每个 `events[t]` 为真的 `(col, t)`，在偏移 `−before..=after` 上记录 `close[t+k]/close[t] − 1`，越界补 NaN（边界事件保留而不是丢弃）；输出行主序，一事件一行。`mean_path`/`median_path` 按偏移聚合，`hit_rate(offset)` 是该偏移上 `> 0` 的比例（恰好 0 不算命中）。它**读未来**，只做分析，不能喂给信号。

## 相关

- [phasor-analysis：分析层](../crates/phasor-analysis.md)：所属 crate；类图里的 `EventPaths` 就是这里的输出。
- [延迟与前视偏差](../concepts/look-ahead.md)：为什么读未来的结果不能喂给信号。
- [机器学习标签](../algorithms/ml-labels.md)：另一类有意读未来、只能做标签或分析的计算。
- [SignalSweep：因子到信号列](../algorithms/signal-sweep.md)：事件通常就是这里生成的信号列。
- [指标公式表](../reference/metrics.md)：事件之外的整段收益指标。
