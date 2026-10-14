---
title: call_seq=auto：共享账户内的执行顺序
kind: algorithm
summary: Auto 用估算的订单金额给组内各列排序，插入排序稳定升序、NaN 最后，让卖单释放的现金先于买单可用。
parent: phasor-sim
related: [bar-loop, size-cascade, short-margin, parallel-simulation, buy-leg]
tags: [simulation, ordering, cash-sharing]
sources:
  - crates/phasor-sim/src/groups.rs:213-322
  - crates/phasor-sim/tests/parity_cash_sharing.rs
order: 1002
---

# call_seq=auto：共享账户内的执行顺序

共享现金的账户组里，同一根 K 线上各列订单的执行先后会改变每笔单能用多少现金。`CallSeq` 决定这个顺序：`Default` 按布局，`Explicit` 由调用方给，`Auto` 按估算的订单金额排序。排序发生在[一根 K 线](../algorithms/bar-loop.md)的第一遍末尾。

## 位置

- `crates/phasor-sim/src/groups.rs:213-249`（`CallSeq::resolve`）
- `crates/phasor-sim/src/groups.rs:262-297`（`approx_order_value`）
- `crates/phasor-sim/src/groups.rs:309-322`（`insert_argsort`）

## 估算订单金额：approx_order_value

共享账户里，一根 K 线先卖后买才能让买单用上卖单释放的现金。`Auto` 用**估算的订单金额**排序：

```text
approx_order_value(size, size_type, direction, cash, position, free_cash, val_price, value):
    ShortOnly → size = −size
    asset_value = position × val_price
    Amount        → size × val_price
    Value         → size
    Percent       → size ≥ 0 ? size × cash
                  : LongOnly ? size × asset_value
                  : size × (2 × max(asset_value, 0) + max(free_cash, 0))    平空要买回 + 释放保证金，所以多头侧翻倍
    TargetAmount  → size × val_price − asset_value
    TargetValue   → size − asset_value
    TargetPercent → size × value − asset_value
正 = 花钱，负 = 回钱；估不出来 → NaN
```

各 `size_type` 在内核里怎样换算成数量见[数量换算链与 percent](../algorithms/size-cascade.md)，`free_cash` 与平空释放的保证金见[空头保证金](../concepts/short-margin.md)。

## 稳定排序：insert_argsort

然后 `insert_argsort`（309–322）：升序、稳定、**NaN 排最后**（"不知道值多少钱"的放在所有回钱的之后），用插入排序因为一组只有几个标的。它是逐字照参考实现写的，包括"已经在位的 NaN 总是右移而不比较"这个细节。
`CallSeq::Default` 按布局顺序；`Explicit` 从列主序的 `&[usize]` 读组内下标。

## 相关

- [phasor-sim：模拟循环](../crates/phasor-sim.md)：所属 crate；`Groups` 与 `CallSeq` 的类图。
- [from_orders 的一根 K 线](../algorithms/bar-loop.md)：排序结果 `seq` 决定第二遍的执行顺序。
- [数量换算链与 percent](../algorithms/size-cascade.md)：估算金额对应的内核换算规则。
- [空头保证金：debt 与 free_cash](../concepts/short-margin.md)：`Percent` 平空分支里 `free_cash` 的来历。
- [buy：买入、平空与 lock_cash 预算](../algorithms/buy-leg.md)：排在后面的买单最终怎样用到卖单释放的现金。
