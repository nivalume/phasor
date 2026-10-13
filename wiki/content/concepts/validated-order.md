---
title: ValidatedOrder：按列提升参数校验
kind: concept
summary: 只看整列参数的校验每列做一次，之后每根 K 线只改四个字段；违例带着走，到原本报错的位置才报。
parent: barq-exec
related: [execute-order-pipeline, flex-array, bar-loop, benchmarks, errors-vs-rejections]
tags: [performance, validation]
sources:
  - crates/barq-exec/src/kernel.rs:416-526
  - benchmarks/README.md
order: 806
---

# ValidatedOrder：按列提升参数校验

撮合内核里唯一的性能结构。它不改变任何可观测结果（同样的成交、同样的拒单、同一位置的同一个错误），只把每根 K 线都在重复的参数校验挪到列循环外面。

## 位置

- `crates/barq-exec/src/kernel.rs:416-526`：`param_violation` 与 `ValidatedOrder`
- `crates/barq-exec/src/kernel.rs:456-462`：收益的来源
- `crates/barq-exec/src/kernel.rs:468-474`：为什么构造不能失败

## 性能结构

14 项校验里有 7 项只看整列通常不变的字段（fees、slippage、min/max_size、granularity、reject_prob、cash_limit）。
`ValidatedOrder::new` 做一次，之后每根 K 线只 `set_bar` 四个会变的字段（size、price、size_type、direction）。
在稀疏工作负载上值 12–15%（`kernel.rs:456-462`），`benchmarks/README.md` 记录了 +30% 稀疏 / +22% 密集。
两个循环都用它（[barq-sim](../crates/barq-sim.md)）。

违例为什么要“带着走”而不是在 `new` 里报错，见 [execute_order 的固定顺序](../algorithms/execute-order-pipeline.md#三个容易漏掉的点)的第一点；模拟循环用 `varies_by_row()` 决定是否每根 K 线重建它，见 [FlexArray 广播与列映射](../concepts/flex-array.md)。

## 相关

- [barq-exec：撮合内核](../crates/barq-exec.md)：`ValidatedOrder` 与 `execute_validated` 是这个 crate 的公开接口。
- [execute_order 的固定顺序](../algorithms/execute-order-pipeline.md)：先 bar 后参数，两半校验的先后决定报哪条消息。
- [FlexArray 广播与列映射](../concepts/flex-array.md)：参数按行变化时才需要每根 K 线重建。
- [from_orders 的一根 K 线](../algorithms/bar-loop.md)：循环里每根 K 线调用 `set_bar` 再 `execute_validated`。
- [基准协议](../tooling/benchmarks.md)：+30% 稀疏 / +22% 密集的测量方法。
- [错误与拒单](../concepts/errors-vs-rejections.md)：参数违例是 `Err`，不是拒单。
