---
title: execute_order 的固定顺序
kind: algorithm
summary: 零钉、忽略检查、校验、换算链、随机拒单、分发：顺序不能换，每一步都有 fixture 钉住。
parent: barq-exec
related: [size-cascade, validated-order, errors-vs-rejections, status-info, float-tolerance, stateless-rng]
tags: [execution, validation, ordering]
sources:
  - crates/barq-exec/src/kernel.rs:559-701
order: 801
---

# execute_order 的固定顺序

`execute_order` 只是把订单包成 `ValidatedOrder` 再交给 `execute_validated`，真正的工作在后者里按固定顺序完成。这个顺序决定了一笔同时有几处毛病的订单最终是被忽略、被拒绝还是报错。

## 位置

- `crates/barq-exec/src/kernel.rs:559-701`：`execute_validated`
- `crates/barq-exec/src/kernel.rs:583-596`：忽略检查
- `crates/barq-exec/src/kernel.rs:598-602`：先 bar 后参数
- `crates/barq-exec/src/kernel.rs:679-693`：随机拒单

## `execute_validated` 的固定顺序

顺序不能换，每一步都有 fixture 钉住。

```mermaid
flowchart TD
  A["零钉：cash/position/debt/free_cash/val_price/value<br/>is_close(v,0) → 0"] --> B{"size 是 NaN？"}
  B -- 是 --> I1["Ignored · SizeNaN"]
  B -- 否 --> C{"price 是 NaN？"}
  C -- 是 --> I2["Ignored · PriceNaN"]
  C -- 否 --> D["validate_bar（7 项：现金、持仓、负债、price>0、方向与持仓一致）"]
  D --> E["validated.check()（7 项参数：fees 有限、slippage≥0、…）"]
  E --> F{"reject_prob>0 且没给 draw？"}
  F -- 是 --> ERR["Err(Invariant)"]
  F -- 否 --> G["数量换算链"]
  G --> H{"draw < reject_prob？"}
  H -- 是 --> R["Rejected · RandomEvent，状态不变"]
  H -- 否 --> J{"order_size > 0？"}
  J -- 是 --> BUY["buy()"]
  J -- "否（0 和 NaN 也走这边）" --> SELL["sell(-order_size)"]
```

换算链的每一级见[数量换算链与 percent](../algorithms/size-cascade.md)；最后的两个分支见 [buy](../algorithms/buy-leg.md) 和 [sell](../algorithms/sell-leg.md)。

## 三个容易漏掉的点

三个容易漏掉的点：

1. **忽略检查在校验之前**（`kernel.rs:583-596`）。一列全是 NaN 的 size 配上非法的 fees，结果是整列 Ignored 而不是报错。这正是 [`ValidatedOrder`](../concepts/validated-order.md) 把违例"带着走"而不是构造时就报错的原因（`kernel.rs:468-474`）。
2. **先 bar 后参数**（`kernel.rs:598-602`）。两组校验独立，但一个订单同时违反两边时报哪条消息由顺序决定，测试 `a_bad_state_outranks_a_bad_parameter` 钉住。
3. **随机拒单在分发之前**（`kernel.rs:679-693`）。参考实现是在买卖之后覆盖结果；这里提前决定，省掉回滚。唯一的可观测差异：被随机拒掉的订单不会再触发"无限做多/做空"那两个 `Err`。

## 相关

- [barq-exec：撮合内核](../crates/barq-exec.md)：`execute_order` 是这个 crate 的唯一入口。
- [数量换算链与 percent](../algorithms/size-cascade.md)：流程图里“数量换算链”那一步的展开。
- [ValidatedOrder：按列提升参数校验](../concepts/validated-order.md)：参数校验那一半为什么能提到循环外、为什么违例要带着走。
- [错误与拒单](../concepts/errors-vs-rejections.md)：哪些出口是 `Err`，哪些是 `Ignored` / `Rejected`。
- [拒单原因 StatusInfo](../reference/status-info.md)：每个出口对应的原因和代码行。
- [无状态随机数 SplitMix64](../concepts/stateless-rng.md)：随机拒单用的 `draw` 由 `(seed, row, col)` 决定。
