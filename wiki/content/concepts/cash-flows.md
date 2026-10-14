---
title: 成交之外的现金流
kind: concept
summary: 入金、分红与收益在固定时点入账，负数截到可用现金为止，实际入账额写成 Deposit/Dividend/Earning 记录。
parent: phasor-sim
related: [bar-loop, rebuild-chain, phasor-records, short-margin, float-tolerance]
tags: [cash, records, accounting]
sources:
  - crates/phasor-sim/src/cash.rs
  - crates/phasor-records/src/cash.rs
order: 1007
---

# 成交之外的现金流

分析层只从记录重建余额，所以任何改变现金的事件都必须是一条记录。成交由内核产生，入金、分红、收益则由 `cash.rs` 在[一根 K 线](../algorithms/bar-loop.md)的开盘和收盘入账，并写成 `CashRecords`（存放在 `OrderRecords.cash` 里，见 [phasor-records](../crates/phasor-records.md)）；[从记录重建净值](../algorithms/rebuild-chain.md)读的就是它们。

## 位置

- `crates/phasor-sim/src/cash.rs`（`is_off`、`apply`、`at_open`、`earn`；顶部的时序表）
- `crates/phasor-records/src/cash.rs`（`CashRecords`）

## 入账时点与读取位置

| 输入 | 入账时点 | 读取位置 | 记录 |
|---|---|---|---|
| `cash_deposits` | 开盘，先于定价与定量 | 组首列（同 `init_cash`） | `Deposit` |
| `cash_dividends` | 开盘，入金之后；每股金额 × 开盘持仓（空头支付） | 每列 | `Dividend` |
| `cash_earnings` | 收盘，所有订单之后，安静的 K 线也入账 | 每列 | `Earning` |

## 跳过、截断与内联

`is_off`：标量 0 或 NaN → 整个输入跳过，每组只判一次，所以不用时零开销。
`apply`：负数最多扣到 `max(free_cash, 0)`，两个余额都用 `add` 更新，记录里存**实际**入账的金额。`±inf` 是 `Err`，NaN 是"无流"。
`at_open`/`earn` 标了 `#[inline(never)]`：循环里活跃变量比寄存器多，把分支挪出去每根 K 线省几条指令。

`free_cash` 为什么可能是负的、负的时候为什么一分钱也扣不出来，见[空头保证金：debt 与 free_cash](../concepts/short-margin.md)；`add` 的容差语义见[浮点容差与零钉](../concepts/float-tolerance.md)。

## 相关

- [phasor-sim：模拟循环](../crates/phasor-sim.md)：所属 crate；`cash.rs` 是 vectorbt 没有的 phasor 新增部分。
- [from_orders 的一根 K 线](../algorithms/bar-loop.md)：`at_open` 与 `earn` 在时序图里的位置。
- [phasor-records：记录与派生视图](../crates/phasor-records.md)：`CashRecords` 的结构与它随 `OrderRecords` 一起合并的方式。
- [从记录重建净值](../algorithms/rebuild-chain.md)：分析层怎样把这些记录重新加回余额。
- [空头保证金：debt 与 free_cash](../concepts/short-margin.md)：出金截断所依据的 `free_cash`。
