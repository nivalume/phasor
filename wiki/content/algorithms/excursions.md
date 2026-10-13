---
title: MAE / MFE 与 edge ratio
kind: algorithm
summary: 每笔交易在进出场区间内逐行跟踪最高与最低价，得到最大不利与最大有利偏移，edge_ratio 是两者收益率均值之比。
parent: barq-records
related: [trade-views, metrics, drawdowns, further-reading]
tags: [trades, excursions, risk]
sources:
  - crates/barq-records/src/excursions.rs:57-146
order: 905
---

# MAE / MFE 与 edge ratio

MAE（maximum adverse excursion，最大不利偏移）与 MFE（maximum favorable excursion，最大有利偏移）衡量一笔交易在持有期间曾经亏到多深、赚到多高，与最后在哪里出场无关。它们建立在[三种交易视图](../algorithms/trade-views.md)产出的 `TradeRecords` 之上，是 barq 相对参考实现新增的派生视图。

## 位置

- `crates/barq-records/src/excursions.rs:67-127`（`excursions`）
- `crates/barq-records/src/excursions.rs:129-146`（`edge_ratio`）

## 计算

每笔交易在 `entry..=exit` 上（`Open` 以状态为准延伸到最后一根）：`hi = lo = entry_price` 起步，逐行 `hi = max(hi, high 或 close)`、`lo = min(lo, low 或 close)`；多头 `mfe = hi − entry`、`mae = lo − entry`，空头镜像；再除以 `entry_price` 得收益率版本。进场与出场那根的整个区间都算进去，对 MAE 偏保守。`edge_ratio = mean(mfe_ret) / mean(|mae_ret|)`。

## 相关

- [barq-records：记录与派生视图](../crates/barq-records.md)：所属 crate；`Excursions` 在类图里由 `TradeRecords` 派生。
- [三种交易视图](../algorithms/trade-views.md)：输入的交易记录，选哪种视图决定"一笔交易"是什么。
- [指标公式表](../reference/metrics.md)：分析层的其他每笔交易指标。
- [延伸资料](../reference/further-reading.md)：Sweeney 的 *Maximum Adverse Excursion* 与 Faith 的 E-ratio。
