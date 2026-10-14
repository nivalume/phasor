---
title: 信号排名与区间
kind: algorithm
summary: signal_rank 与 partition_rank 共用一张走表，段内 reset 立刻重新编号；partition_ranges 把每段落成一条 RangeRecords。
parent: phasor-signals
related: [signal-generation, trade-views, drawdowns, phasor-records]
tags: [signals, rank, ranges]
sources:
  - crates/phasor-signals/src/rank.rs
  - crates/phasor-signals/src/ranges.rs
order: 402
---

# 信号排名与区间

[phasor-signals](../crates/phasor-signals.md) 把一列布尔信号看成若干个"分区"（连续 `true` 的段）：`rank.rs` 给信号或分区编号，`ranges.rs` 把段变成区间记录。区间的 `duration` 与[三种交易视图](../algorithms/trade-views.md)、[回撤段](../algorithms/drawdowns.md)用同一条规则。

## 位置

- `crates/phasor-signals/src/rank.rs:39-80`（`rank_columns` 与两个 `step`）
- `crates/phasor-signals/src/rank.rs:87`、`121`（`signal_rank`、`partition_rank`）
- `crates/phasor-signals/src/rank.rs:148`、`187`（`nth_index`、`normalized_average_index`）
- `crates/phasor-signals/src/rank.rs:257-266`（测试 `a_reset_inside_a_run_renumbers_from_that_bar`）
- `crates/phasor-signals/src/ranges.rs:33-111`（`RangeRecords`、`duration`、`mean_closed_duration`）
- `crates/phasor-signals/src/ranges.rs:132`、`167`、`189`（`partition_ranges`、`between_ranges`、`between_partition_ranges`）

## 排名：`rank.rs`

**`rank.rs`**：分区（run）= 连续 `true` 的最大段。`signal_rank` 给信号编号、`partition_rank` 给分区编号，`false` 处写 −1。共用走表 `rank_columns`：状态 `counter, reset_row, prev_run_end, run_start, seen_false`。`signal_rank` 的 `step`：`reset_here = reset_row > prev_run_end && max(reset_row, run_start) == i`（`> prev_run_end` 的守卫防一个陈旧的 reset 在下一段再触发）；`ResetEachRun` 时每段从 0；**段内的 reset 立刻从那根重新编号**（`XXXX` 在第 1 根 reset → `[0,0,1,2]`）。`LeadingRun::Skip` 整段丢掉开头已在进行的 run。

## 区间：`ranges.rs`

**`ranges.rs`**：`partition_ranges` 每段一条，闭合段 `end_idx` 是段后第一个 `false`（排他），末尾未结束的段 `Open` 且 `end_idx = n_rows − 1`（包含）；`duration = end − start`（closed）/ `+1`（open）——与交易、回撤同一条规则；`mean_closed_duration` 只平均闭合段。

## 相关

- [phasor-signals：信号工具箱](../crates/phasor-signals.md)：所属 crate；类图里的 `RangeRecords`。
- [信号清洗与出场生成](../algorithms/signal-generation.md)：排名和区间通常作用在清洗或生成后的信号上。
- [三种交易视图](../algorithms/trade-views.md)：未平仓交易的 `duration` 同样加 1，与这里的 `Open` 区间是同一条规则。
- [回撤段识别](../algorithms/drawdowns.md)：Recovered 用 `end − start`、Active 用 `end − start + 1`，与这里的闭合/未结束是同一条时长规则。
- [phasor-records：记录与派生视图](../crates/phasor-records.md)：`RangeRecords` 沿用记录的结构体数组布局，`id` 不存储，就是记录下标。
