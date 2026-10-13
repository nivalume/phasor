---
title: 信号冲突解析链与 signals_to_size
kind: algorithm
summary: signals.rs 里顺序固定的纯函数先解止损、进出与方向冲突，再由 signals_to_size 把幸存信号变成有符号数量。
parent: barq-sim
related: [prepare-order, stop-engine, size-cascade, parity-testing, short-margin]
tags: [signals, conflicts, sizing]
sources:
  - crates/barq-sim/src/signals.rs:1-401
  - crates/barq-sim/tests/parity_signals.rs
  - tests/fixtures/signals.tsv
order: 1005
---

# 信号冲突解析链与 signals_to_size

一根 K 线上四个布尔信号（多头进出、空头进出）可能同时为真，也可能与当前持仓相矛盾。[from_signals 的决策顺序](../algorithms/prepare-order.md)把它们交给 `signals.rs` 里一串顺序固定的纯函数：先决定哪些信号幸存，再算出一个有符号数量。因为是纯函数，它们可以脱离循环、逐个与参考实现穷举对照。

## 位置

- `crates/barq-sim/src/signals.rs:85-120`（`from_stop`）
- `crates/barq-sim/src/signals.rs:129-172`（`resolve_signal_conflict`）
- `crates/barq-sim/src/signals.rs:176-208`（`resolve_dir_conflict`）
- `crates/barq-sim/src/signals.rs:217-258`（`resolve_opposite_entry`）
- `crates/barq-sim/src/signals.rs:290-401`（`signals_to_size`）

## 解析链

四个纯函数，顺序固定（文件注释 13–20 行：先解方向冲突再解进出冲突，幸存的信号会不同）。全部用 5,088 行 fixture 穷举对照。

| 阶段 | 函数 | 解决什么 | 规则摘要 |
|---|---|---|---|
| 1 | `from_stop(position, mode, acc)` 85–120 | 止损触发后发什么信号 | 由持仓方向决定：`Close` → 退出 + 关闭累加；`CloseReduce` → 退出；`Reverse` → 反向进场 + 关闭累加；`ReverseReduce` → 反向进场 |
| 2 | `resolve_signal_conflict(position, entry, exit, dir, mode)` 129–172 | 同方向进出同时触发 | `Entry`/`Exit` 二选一；`Adjacent`/`Opposite` 相对于持仓，空仓时两者都丢；`Ignore` 都丢 |
| 3 | `resolve_dir_conflict(position, long_entry, short_entry, mode)` 176–208 | 多头进场和空头进场同时触发 | `Long`/`Short`/`Adjacent`（跟持仓同向）/`Opposite`/`Ignore` |
| 4 | `resolve_opposite_entry(position, signals, mode, acc)` 217–258 | 进场信号与持仓相反 | `Ignore` 丢掉；`Close` 改成退出 + 关闭累加；`CloseReduce` 改成退出；`Reverse` 保留 + 关闭累加；`ReverseReduce` 不变 |
| 5 | `signals_to_size(position, signals, size, size_type, acc, val_price)` 290–401 | 算出有符号数量 | 见下表 |

阶段 1 只在止损触发时走（见[止损引擎](../algorithms/stop-engine.md)），阶段 2–4 只在有进场信号时走；fixture 与对照方法见[差分测试与 fixture](../tooling/parity-testing.md)。

## signals_to_size：从信号到有符号数量

`signals_to_size` 只接受 `Amount | Value | Percent`，`size` 必须非负（方向由信号表达）。`adds = Both|AddOnly`，`removes = Both|RemoveOnly`：

| 持仓 | 信号 | removes / adds | 结果 |
|---|---|---|---|
| 多 | short_entry | removes | `−size`（减仓） |
| 多 | short_entry | 否则 | 反向：`−\|position\| − size`（`Value` 时 `size/val_price`），类型改 `Amount`；`Percent` 反向是 `Err` |
| 多 | long_exit | removes / 否则 | `−size` / `−\|position\|`，`LongOnly` |
| 多 | long_entry | adds | `+size`，`LongOnly`；否则 0 |
| 空 | 镜像 | | `ShortOnly` |
| 平 | long_entry / short_entry | | `+size` / `−size` |

反向是**一笔**订单（平掉 + 开反向），这就是百分比不能反向的原因："百分比是相对于被平掉的仓位还是新开的仓位？"

## 相关

- [barq-sim：模拟循环](../crates/barq-sim.md)：所属 crate；这些函数与 vectorbt `*_nb` 函数的逐一对应。
- [from_signals 的决策顺序](../algorithms/prepare-order.md)：解析链在流程图里的位置与调用条件。
- [止损引擎：触发、跟踪与重新布防](../algorithms/stop-engine.md)：阶段 1 `from_stop` 的输入从哪来。
- [数量换算链与 percent](../algorithms/size-cascade.md)：`signals_to_size` 产出的 `Amount`/`Value`/`Percent` 在内核里怎样换算。
- [差分测试与 fixture](../tooling/parity-testing.md)：5,088 行穷举 fixture 的生成与重放。
