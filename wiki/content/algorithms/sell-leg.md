---
title: sell：卖出与开空
kind: algorithm
summary: 先算可卖数量的上限（只做多时是持仓，允许做空时看 free_cash 还能空多少），持仓卖穿 0 的那一段记成空头负债。
parent: phasor-exec
related: [short-margin, buy-leg, size-cascade, status-info, float-tolerance]
tags: [execution, selling, shorting]
sources:
  - crates/phasor-exec/src/kernel.rs:223-379
order: 804
---

# sell：卖出与开空

换算后 `order_size` 不为正的订单（包括 0 和 NaN）都以 `sell(-order_size)` 进入这里。它先限定这笔单最多能卖多少，再结算卖出所得；持仓穿过 0 的那一段变成空头，记进 `debt` 并占用 `free_cash`。

## 位置

- `crates/phasor-exec/src/kernel.rs:228-379`：`sell`
- `crates/phasor-exec/src/kernel.rs:268-291`：`lock_cash` 时在上限里乘掉 `percent`
- `crates/phasor-exec/src/kernel.rs:349-366`：开空时记 `debt`、扣 `free_cash`

## 伪代码

输入 `size > 0`。

```text
adj_price = price × (1 − slippage)

size_limit =
  LongOnly                         → min(position, size)                # 只能卖持有的
  lock_cash 或 (size=inf 且有 percent) →
      long_size  = max(position, 0)
      long_cash  = long_size × adj_price × (1 − fees)                  # 卖多头能回多少
      total_free = add(free_cash, long_cash)
      total_free ≤ 0 → position ≤ 0 ? Rejected·NoCashShort : 上限 = long_size
      否则 max_short = add(total_free, -fixed) / (adj_price × (1 + fees))
           limit = long_size + max_short；limit ≤ 0 → Rejected·CantCoverFees
      lock_cash 时在这里就把 percent 用掉（置 NaN）
  其他                              → size

percent 未用掉 → size_limit *= percent
size_limit > max_size → allow_partial ? = max_size : Rejected·MaxSizeExceeded
Short/Both: size_limit 无限 → Err（无限做空）
LongOnly:   position == 0 → Rejected·NoOpenPosition
粒度取整；is_close(size_limit,0) → Ignored·SizeZero；< min_size → MinSizeNotReached；部分成交检查

acq_cash  = size_limit × adj_price
fees_paid = acq_cash × fees + fixed
final_acq = add(acq_cash, -fees_paid)；< 0 → Rejected·CantCoverFees

new_cash     = cash + final_acq
new_position = add(position, -size_limit)
new_position < 0（有一部分变成空头）：
    short_size  = position < 0 ? size_limit : |new_position|            # 只有净空头部分产生负债
    short_value = short_size × adj_price
    new_debt      = debt + short_value
    new_free_cash = add(free_cash, add(final_acq, -2·short_value))
否则：new_free_cash = free_cash + final_acq
```

开空之后 `debt` 和 `free_cash` 为什么这样变、持仓在空仓 / 多头 / 空头之间怎么移动，见[空头保证金：debt 与 free_cash](../concepts/short-margin.md)；平空的释放在 [buy](../algorithms/buy-leg.md) 里。

## 相关

- [phasor-exec：撮合内核](../crates/phasor-exec.md)：`sell` 是 `execute_validated` 的两个分支之一。
- [空头保证金：debt 与 free_cash](../concepts/short-margin.md)：`new_position < 0` 分支里 `2 × short_value` 的由来。
- [buy：买入、平空与 lock_cash 预算](../algorithms/buy-leg.md)：另一个分支；平空时按 `debt / |position|` 释放。
- [数量换算链与 percent](../algorithms/size-cascade.md)：负比例和 `size = −inf` 怎样变成 `sell` 的 `percent`。
- [拒单原因 StatusInfo](../reference/status-info.md)：`NoCashShort`、`CantCoverFees` 等在 `sell` 里的位置。
- [浮点容差与零钉](../concepts/float-tolerance.md)：余额更新用 `add`，对零的比较是精确比较。
