---
title: 三种交易视图
kind: algorithm
summary: entry_trades、exit_trades、positions 用同一个容差 PnL 公式，按不同规则把成交配对成交易，只在 Σ pnl 上一致。
parent: phasor-records
related: [excursions, float-tolerance, colmap, metrics, drawdowns, rebuild-chain]
tags: [trades, pnl, records]
sources:
  - crates/phasor-records/src/trades.rs:11-19
  - crates/phasor-records/src/trades.rs:78-94
  - crates/phasor-records/src/trades.rs:368-486
  - crates/phasor-records/src/trades.rs:502-628
  - crates/phasor-records/src/trades.rs:639-697
  - crates/phasor-records/tests/parity_trades.rs
order: 903
---

# 三种交易视图

成交记录说的是"哪天买卖了多少"，交易统计要的是"一笔交易赚了多少"。从前者到后者有不止一种配对方法：按进场成交算、按出场成交算、按整段持仓算。`trades.rs` 三种都实现，分析层默认用出场视图。

## 位置

- `crates/phasor-records/src/trades.rs:78-94`（`trade_stats`，PnL 公式）
- `crates/phasor-records/src/trades.rs:368-486`（`entry_trades`）
- `crates/phasor-records/src/trades.rs:502-628`（`exit_trades`）
- `crates/phasor-records/src/trades.rs:639-697`（`positions`）
- `crates/phasor-records/src/trades.rs:11-19`（三种视图的对比表）

## 共用的 PnL 公式

共用的 PnL 公式（78–94）：

```text
entry_val = size × entry_price；exit_val = size × exit_price
val_diff  = add(exit_val, −entry_val)        ← 容差加法：盈亏平衡的往返精确为 0
Short 则 val_diff = −val_diff
pnl = val_diff − entry_fees − exit_fees
ret = pnl / entry_val
```

没有 `add`，一次平手往返的残差是 5.7e-14，会被 `pnl > 0` 数成一次"盈利"。`add` 的容差规则见[浮点容差与零钉](../concepts/float-tolerance.md)。

## 前置条件与分配规则

前置条件：记录按列分组、行递增（`column_ranges` 要求 `ColMap` 连续，否则 `Err`）；`size > 0`、`price > 0`。`Buy` 开多、`Sell` 开空；与持仓同向的成交是进场，否则是出场。

`ColMap` 的连续性见 [ColMap 稳定计数排序与 MappedArray](../algorithms/colmap.md)。

| 视图 | 一条记录 = | 分配规则 | 关键细节 |
|---|---|---|---|
| `entry_trades` 368–486 | 一笔**进场**成交 | 出场按 FIFO 回填到各进场 | 所有进场共享一个加权平均出场价和一个 `exit_idx`；出场费按进场数量比例分摊；一笔成交同时平仓又反向时，平仓部分取 `remaining/size × fees`，剩余部分开新仓且 `first_entry_size = opened`（同一个成交下标成为新仓的 `first`，`flush` 用它避免被按全额计数） |
| `exit_trades` 502–628 | 一笔**出场**成交 | 进场取加权平均 | `entry_price = entry_gross_sum / entry_size_sum`；出场数量与持仓差一个 ULP 时钉成整仓；部分出场后三个进场累加量按 `kept = (entry_size_sum − size)/entry_size_sum` 缩放，剩余仓位保持平均成本并卸掉已归属的费用；反向时平掉的部分带走全部 `entry_fees_sum` |
| `positions` 639–697 | 一个 `parent_id` | 把上面任一视图里连续相同 `parent_id` 的记录合并 | 单条记录 `push_copy` 逐位复制（避免 `5.128 × 58.78 / 5.128 != 58.78`）；多条时数量求和、价格按数量加权、费用求和、方向与状态取最后一条 |

## 未平仓位与视图之间的差异

未平仓位在末尾按最后一根 `close` 标记：`exit_idx = n_rows − 1`、无出场费、状态 `Open`。
三种视图只在 **Σ pnl** 上一致，数量、每笔 `ret`、胜率都可以不同（`trades.rs:11-19`）。`parent_id` 跨列全局递增；`positions` 的输出 `parent_id` 等于输出下标，所以 positions 的 positions 是恒等。

## 相关

- [phasor-records：记录与派生视图](../crates/phasor-records.md)：所属 crate；默认视图是 `ExitTrades`、未平仓交易如何计入指标等陷阱。
- [MAE / MFE 与 edge ratio](../algorithms/excursions.md)：在这些交易记录上计算持有期间的偏移。
- [浮点容差与零钉](../concepts/float-tolerance.md)：`add` 让平手往返的 PnL 精确为 0。
- [ColMap 稳定计数排序与 MappedArray](../algorithms/colmap.md)：`column_ranges` 依赖的按列分组。
- [指标公式表](../reference/metrics.md)：胜率、期望等每笔交易统计随视图而变。
