---
title: 错误与拒单
kind: concept
summary: Err 只表示不变量被破坏、整个运行停止；订单成交不了是正常结果，以 Ignored 或 Rejected 加原因返回。
parent: phasor-core
related: [status-info, execute-order-pipeline, validated-order, phasor-exec, conventions]
tags: [errors, rejections, invariants]
sources:
  - crates/phasor-core/src/error.rs:1-58
  - crates/phasor-exec/src/lib.rs:12-18
  - crates/phasor-exec/src/kernel.rs:381-452
  - crates/phasor-sim/src/from_orders.rs:489-495
order: 105
---

# 错误与拒单

phasor 区分两种“失败”。一种是调用方给了不可能的输入，或者模拟依赖的不变量被破坏，这时返回 `Err`，运行停止；另一种是订单在当时的账户状态下成交不了，这是正常结果，带着原因原样返回。[phasor-core](../crates/phasor-core.md) 的 `Error` 只用于前一种，后一种是 [phasor-exec](../crates/phasor-exec.md) 撮合内核的普通返回值。

## 位置

- `crates/phasor-core/src/error.rs:1-11`：模块文档
- `crates/phasor-core/src/error.rs:18-58`：`Error` 的五个变体
- `crates/phasor-exec/src/lib.rs:12-18`：撮合内核的“两种失败”
- `crates/phasor-exec/src/kernel.rs:381-452`：`validate_bar` 与 `param_violation`
- `crates/phasor-sim/src/from_orders.rs:489-495`、`crates/phasor-sim/src/from_signals.rs:870-876`：`raise_reject`

## 拒单不是错误

这是全仓库的约定之一（[贯穿全仓库的约定](../guides/conventions.md)）：`Err(Error::Invariant)` 表示不变量被破坏（负现金、坏参数），整个运行停止；撮合不成功只是 `OrderResult { status: Rejected/Ignored, status_info }`。

[ARCHITECTURE §3.5](../../../docs/ARCHITECTURE.zh-CN.md) 的说法：

- `Err(Error::{Broadcast, FlexLen, UnknownLevel, LevelWidth, Invariant})` 表示不变量被破坏（负现金、非法参数），运行停止。
- **拒单从来不是 `Err`**：它是 `OrderResult`，`status = Ignored | Rejected`，`status_info` 给出原因（[拒单原因 StatusInfo](../reference/status-info.md)），并记录在日志中（[ARCHITECTURE §9.4](../../../docs/ARCHITECTURE.zh-CN.md)，见 [phasor-records](../crates/phasor-records.md)）。

## 代码里的两段说明

`crates/phasor-core/src/error.rs` 的模块文档：

> Note the distinction this type draws, which runs through the whole engine:
> an `Error` is an *invariant violation* — the caller handed us something
> impossible (mismatched shapes, a level that does not exist, negative
> cash). An order that simply cannot be filled is **not** an error; it is a
> rejected order carrying a status code, returned through the normal path.

`crates/phasor-exec/src/lib.rs` 的模块文档，“Two kinds of failure”一节：

> A *rejected* order is a normal outcome: it returns an `OrderResult` with
> `OrderStatus::Rejected` and a `StatusInfo` explaining why, leaving the
> state untouched. An *invariant violation* — negative cash, a non-finite
> position, a long-only column holding a short — is an `Err`. Conflating the
> two is how a simulation silently produces a plausible, wrong equity curve.

也就是：被拒的订单不改动账户状态，只带回 `StatusInfo`；负现金、非有限的持仓、只做多的列持有空头这类情况才是 `Err`。两者混为一谈，模拟就会无声地产出一条看似合理、实则错误的净值曲线。

## 五个 `Error` 变体从哪来

| 变体 | 含义 | 典型来源 |
|---|---|---|
| `Broadcast { left, right }` | 两个需要一起广播的数组形状对不上 | `phasor-generic`、`phasor-signals`、`phasor-records` 等处的形状检查 |
| `FlexLen { kind, actual, expected }` | 数组长度与它声明的轴不符 | `Matrix::from_column_major`，`FlexArray` 的 `per_column` / `per_row` / `full` |
| `UnknownLevel { name, available }` | 按不存在的层级名查列标签 | `Columns::level`、`label`、`group_indices` |
| `LevelWidth { name, actual, expected }` | 各层级描述的列数不一致 | `Columns::new` |
| `Invariant(String)` | 模拟依赖的数值不变量被破坏 | `FlexArray::mapped` 的形状检查；撮合内核的 `validate_bar` / `param_violation`、`reject_prob > 0` 却没给随机数、无限做多或做空 |

## 撮合内核里的分界

- **忽略检查在校验之前**：一列全是 NaN 的 size 配上非法参数，结果是整列 `Ignored` 而不是 `Err`。这一点和其余两个顺序细节见 [execute_order 的固定顺序](../algorithms/execute-order-pipeline.md)。
- **参数违例带着走**：`ValidatedOrder::new` 不会失败，违例在原本会报错的位置才变成 `Err`，见 [ValidatedOrder：按列提升参数校验](../concepts/validated-order.md)。
- **`raise_reject` 是调用方的选择**：`Order.raise_reject` 为真时，两条模拟循环把 `Rejected`（不含 `Ignored`）改写成 `Err(Error::Invariant)`，消息里带上行、列和原因；内核本身仍然只返回 `OrderResult`。

[phasor-core 的阅读陷阱](../crates/phasor-core.md#阅读陷阱)里还有两条边界情况：`FlexArray` 的公开变体可以绕过检查构造，长度错了会在模拟中途 panic 而不是返回 `Error`；`phasor-data` 不用 `phasor_core::Error`，用 `ArrowError`。

## 相关

- [phasor-core：基础类型](../crates/phasor-core.md)：`Error` 与 `Result` 定义在这里。
- [拒单原因 StatusInfo](../reference/status-info.md)：14 种拒单原因各在哪一行触发。
- [execute_order 的固定顺序](../algorithms/execute-order-pipeline.md)：忽略检查、校验、随机拒单的先后决定一笔订单报哪种结果。
- [ValidatedOrder：按列提升参数校验](../concepts/validated-order.md)：参数违例为什么延后到原位置再报。
- [phasor-exec：撮合内核](../crates/phasor-exec.md)：“两种失败”是这个 crate 模块文档的第一节。
- [贯穿全仓库的约定](../guides/conventions.md)：“拒单不是错误”是全仓库的约定之一。
