---
title: 回撤段识别
kind: algorithm
summary: find_drawdowns 逐列扫描净值，用峰、谷两个游标切出回撤段，Recovered 与 Active 两种状态用不同的时长规则。
parent: phasor-records
related: [trade-views, metrics, rebuild-chain, excursions]
tags: [drawdowns, records, risk]
sources:
  - crates/phasor-records/src/drawdowns.rs:247-324
  - crates/phasor-records/src/drawdowns.rs:137-206
  - crates/phasor-records/tests/parity_drawdowns.rs
order: 904
---

# 回撤段识别

最大回撤只是一个数；回撤段记录则把每一次"从峰值跌下去、再回到峰值"的过程单独记下来：峰在哪、谷在哪、何时恢复、还没恢复的算不算。`find_drawdowns` 在一列净值（或任何时间序列，例如[从记录重建的净值](../algorithms/rebuild-chain.md)）上一次扫描产出这些记录。

## 位置

- `crates/phasor-records/src/drawdowns.rs:247-324`（`find_drawdowns`）
- `crates/phasor-records/src/drawdowns.rs:137-206`（访问器：`duration`、`drawdown` 等）

## 扫描

```text
每列：started=false，peak=(−1, ts[0])，valley=peak
对每个非 NaN 的 bar i：
    cur ≥ peak_val（或 peak 是 NaN）：
        started → 记一段 Recovered（等于峰值也算恢复）
        否则     → 延长峰值（平顶延长峰值，不开新段）
    否则 started → cur < valley_val 时更新谷底
    否则          → 开一段：started=true，valley=(i, cur)
    最后一根仍 started → 记一段 Active
    记录后：peak = valley = (i, cur)   ← 关闭一段的那根是下一段的峰
```

## 时长、深度与末尾 NaN

`start_idx = peak_idx + 1`。时长规则：`duration = end − start`（Recovered，结束那根已回到水面之上）/ `end − start + 1`（Active，最后一根仍在水下）；一条规则套两种会把平均偏掉一根。`drawdown = (valley − peak)/peak`。陷阱：末尾是 NaN 的列，"最后一根"的判断不会执行，尚未结束的那段会被**丢弃**。

## 相关

- [phasor-records：记录与派生视图](../crates/phasor-records.md)：所属 crate；`DrawdownRecords` 的字段与 `generic/nb.py::drawdowns_nb` 的对应。
- [指标公式表](../reference/metrics.md)：最大回撤、回撤时长等指标从这些记录取值。
- [从记录重建净值](../algorithms/rebuild-chain.md)：通常输入的那条净值曲线。
- [三种交易视图](../algorithms/trade-views.md)：同一 crate 里另一类派生记录，同样遵守"id 就是下标"。
