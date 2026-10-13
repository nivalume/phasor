---
title: 从记录重建净值
kind: algorithm
summary: 现金、持仓、净值与收益都从订单和现金记录按需派生：asset_flow → assets、cash_flow → cash，再合成 value 与 returns。
parent: barq-analysis
related: [cash-flows, short-margin, drawdowns, metrics, float-tolerance, column-axis]
tags: [analysis, rebuild, records]
sources:
  - crates/barq-analysis/src/series.rs
  - crates/barq-analysis/src/portfolio.rs:534-662
  - crates/barq-analysis/tests/cash_flows.rs
order: 1101
---

# 从记录重建净值

`Portfolio` 只存记录、借来的收盘价、分组和初始现金；这条链把它们按需变回持仓、现金、净值与收益，是["记录是唯一来源"](../guides/conventions.md)在分析层的落地：模拟器里改变现金或持仓的每件事都已经是一条记录，所以重建不需要任何模拟参数。

## 位置

- `crates/barq-analysis/src/series.rs`：6–11 行的链路图；`asset_flow`→…→`returns_with_deposits` 74–380；`free_cash_flow` 191–242
- `crates/barq-analysis/src/portfolio.rs:534-662`：`Portfolio` 上的链路方法（结果缓存在 `Cache` 的 `OnceLock` 里）
- `crates/barq-analysis/src/portfolio.rs:664-764`：敞口家族
- `crates/barq-analysis/tests/cash_flows.rs`：内核日志与重建余额的守恒测试

## 链路

![重建链](../../../docs/img/zh/rebuild.svg)

```text
asset_flow(orders, direction)      每笔成交：signed = ±size；new_position = add(position, size)；
                                    Both 下流量 = size；LongOnly/ShortOnly 下用 long_size/short_size 把反向拆成平仓腿和开仓腿；
                                    同一格累加用 add，所以一根 K 线内平手往返精确为 0；换列时持仓重置（依赖列主序）
assets          = 每列 asset_flow(Both) 的 add 累计
cash_flow       = 每笔成交 −size × price − fees，再加上 CashRecords 的 amount
deposits        = 只取 Deposit 类的流（没有则 None → 无入金路径与旧链逐位一致）
cash            = running = init_cash；running = add(running, flow[t])            按账户组求和后再跑
asset_value     = close × assets；共享账户时 assets == 0 的格子钉 0（防 NaN 收盘价污染整组），再按组求和
value           = cash + asset_value                                            （普通 +，不是 add）
period_return   = (after − before)/before；before == 0 时：after == 0 → 0，否则 ±inf；before < 0 时翻号
returns         = r[t] = value[t] / (value[t−1] + deposit[t]) − 1，第 0 根从 init_cash + deposit[0] 起
```

`add` 的零钉语义见[浮点容差与零钉](../concepts/float-tolerance.md)，"换列时持仓重置"依赖的列主序见[列轴与列主序](../concepts/column-axis.md)；`CashRecords` 来自[成交之外的现金流](../concepts/cash-flows.md)。

## 有入金时的调整

有入金时的三条调整（`portfolio.rs`）：总收益改为 `Π(1 + r) − 1` 连乘而不是 `end/init − 1`；回撤在 `compounded_value = init × Π(1 + r)` 上找（"出金不是回撤，入金不是恢复"）；没有入金时这些路径逐位等于旧链。

## free_cash_flow：回放负债

`free_cash_flow`（191–242）从记录回放内核的 `debt`：开空 `debt += short_size × price`、`diff = add(final_cash, −2 × short_value)`；平空 `avg_entry = debt/|position|`、`diff = add(2 × debt_diff, final_cash)`。`tests/cash_flows.rs` 检查每条日志里内核看到的 `cash`/`free_cash` 等于从记录重建的余额——这是"记录是唯一来源"的守恒测试。

## 敞口家族

敞口家族（664–764）：`allocations = column_asset_value / value[group]`；`long_exposure`/`short_exposure` 分别对 `max(v,0)`、`max(−v,0)` 按组求和再除以总净值；`net = long − short`，`gross_long_short = long + short`；旧的 `gross_exposure` 分母是 `|asset_value| + free_cash`，保持不变。`position_coverage` 是按**标的列**而不是按账户。

## 相关

- [barq-analysis：分析层](../crates/barq-analysis.md)：所属 crate；它的调用链展示 `Cache` 怎样按需填充这些序列。
- [空头保证金：debt 与 free_cash](../concepts/short-margin.md)：`free_cash_flow` 回放的就是内核里 `debt` 与 `free_cash` 的这套账。
- [成交之外的现金流](../concepts/cash-flows.md)：`CashRecords` 与入金从哪里来。
- [回撤段识别](../algorithms/drawdowns.md)：有入金时回撤在 `compounded_value` 上找。
- [指标公式表](../reference/metrics.md)：`returns` 算出来之后的每个指标。
- [三种交易视图](../algorithms/trade-views.md)：`trades` 也是从同一份订单记录派生的视图。
