---
title: from_signals 的决策顺序
kind: algorithm
summary: prepare_order 按固定顺序解析价格与估值价、测止损、读信号并解冲突、定量，最后产出一个 Pending 或 None。
parent: phasor-sim
related: [stop-engine, signal-conflicts, bar-loop, size-cascade, call-seq-auto]
tags: [simulation, signals, pipeline]
sources:
  - crates/phasor-sim/src/from_signals.rs:502-704
  - crates/phasor-sim/src/from_signals.rs:356-385
  - crates/phasor-sim/src/from_signals.rs:711-993
order: 1003
---

# from_signals 的决策顺序

`from_signals` 的循环结构与 [from_orders 的一根 K 线](../algorithms/bar-loop.md)相同，只是第一遍每列多做一步 `prepare_order`：把这根 K 线的价格、估值价、止损状态和四个布尔信号收敛成一个待执行的 `Pending`（或者 `None`，表示这一列这根 K 线什么也不做）。下面的流程图就是它的全部决策顺序。

## 位置

- `crates/phasor-sim/src/from_signals.rs:502-704`（`prepare_order`）
- `crates/phasor-sim/src/from_signals.rs:356-385`（`read_signals`）
- `crates/phasor-sim/src/from_signals.rs:711-993`（`simulate_group`，调用 `prepare_order` 并执行 `Pending`）

## 决策流程

![信号到订单](../../../docs/img/zh/signals.svg)

```mermaid
flowchart TD
  P["① 价格：+inf→close；−inf→本根 open（NaN 则前收）"] --> V["② 估值价 val_price：+inf→price；−inf→前收；NaN 时 ffill<br/>（先于止损，止损成交不改变本根估值）"]
  V --> A{"③ use_stops 且 stops.armed()？"}
  A -- 是 --> R["缺 O/H/L 用 close 重建"] --> SL["先测 SL：stop_price(…, hit_below=true)"]
  SL --> TP["SL 没触发才测 TP"]
  TP --> TR["测完再 trail：多头 sl_price=max(sl_price, high)；空头 =min(sl_price, low)"]
  TR --> B
  A -- 否 --> B{"④ 止损触发了？"}
  B -- 是 --> FS["from_stop(position, upon_stop_exit) → 信号由持仓方向决定<br/>stop_exit_price：StopMarket→price=触发价；StopLimit→同上且无滑点；Close→收盘价；Price→不变"]
  B -- 否 --> RS["read_signals：四个方向信号 OR 上 entries/exits 经 direction 映射<br/>（Both 下 exit = 空头进场）"]
  RS --> Q{"sizes_checked 且无信号？"}
  Q -- 是 --> NONE["返回 None（安静的 K 线，跳过定量）"]
  Q -- 否 --> CF["有进场信号才解冲突：<br/>resolve_signal_conflict（多）→ resolve_signal_conflict（空）<br/>→ resolve_dir_conflict → resolve_opposite_entry"]
  FS --> SZ
  CF --> SZ["⑤ signals_to_size(position, signals, size, size_type, accumulate, val_price)"]
  SZ --> OUT{"size == 0？"}
  OUT -- 是 --> NONE
  OUT -- 否 --> PEND["Pending{size（ShortOnly 翻号）, price, size_type, direction, stop_limit}"]
```

③④ 两步（`stop_price`、先测后 trail、成交后的重新布防与止损单的滑点）展开在[止损引擎：触发、跟踪与重新布防](../algorithms/stop-engine.md)；冲突解析与 ⑤ `signals_to_size` 的规则表在[信号冲突解析链与 signals_to_size](../algorithms/signal-conflicts.md)。

## 相关

- [phasor-sim：模拟循环](../crates/phasor-sim.md)：所属 crate；`Pending`、`Signals`、`Stops` 的类图与 `−inf` 价格、`exits` 语义等阅读陷阱。
- [止损引擎：触发、跟踪与重新布防](../algorithms/stop-engine.md)：流程图 ③④ 的细节。
- [信号冲突解析链与 signals_to_size](../algorithms/signal-conflicts.md)：流程图里 `from_stop`、`resolve_*` 与 ⑤ 的纯函数。
- [from_orders 的一根 K 线](../algorithms/bar-loop.md)：`prepare_order` 之后的排序与执行沿用同一结构。
- [数量换算链与 percent](../algorithms/size-cascade.md)：`Pending` 交给内核后 `size_type` 怎样换算成数量。
