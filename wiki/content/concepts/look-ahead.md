---
title: 延迟与前视偏差
kind: concept
summary: 在 t 收盘算出的值要到 t 收盘后才知道，最早在 t+1 交易；读未来的函数都被标注，几道机制在数据、因子、信号和研究层挡住它。
parent: barq-signals
related: [ml-labels, pit-alignment, strategy-sweep, signal-sweep, research-harness, event-study]
tags: [look-ahead, delay, signals]
sources:
  - crates/barq-generic/src/lib.rs:16-18
  - crates/barq-generic/src/elementwise.rs:5-11
  - crates/barq-signals/src/labels.rs:1-14
  - crates/barq-expr/src/parse.rs:37-51
  - crates/barq/src/sweep.rs:90-104
  - crates/barq/tests/factor_path.rs
  - crates/barq-py/python/barq/strategy.py:36-76
  - research/barq_lab/types/factor_ts/harness.py:143-155
  - research/barq_lab/types/factor_ts/harness.py:429-454
order: 405
---

# 延迟与前视偏差

这是一条贯穿研究层与信号层的红线：**延迟**。在 `t` 的收盘价算出的因子，`t` 才知道，在 `t` 交易就是用了未来数据。所有信号工具默认延迟一根 K 线或把延迟当参数；`labels.rs` 里读未来的函数只能做训练目标，绝不能做信号。

## 延迟：在 t 决定，在 t+1 交易

> **唯一关键的规则：延迟。** 用第 `t` 根 K 线收盘价算出的因子，要等第 `t` 根收盘后才知道；在 `t` 上用它交易就是未来函数。所有辅助函数都默认延迟，或者把延迟作为参数（`crates/barq/tests/factor_path.rs`）。
> —— [ARCHITECTURE §6](../../../docs/ARCHITECTURE.zh-CN.md)

`factor_path.rs` 在策略里手动平移一根（`raw_entries[row] = trend[row - 1] > 0.05`），注释写明这样做是为了把时点决定留在策略里，而不是藏进表达式引擎。Python 侧 `_delay(1)` 配合默认 `price = +inf`（本根收盘）意味着"在 `t` 决定、在 `t+1` 的收盘成交"。

`barq-generic` 的态度是**前视是被标注的，不是被阻止的**：读未来的函数照样存在（打标签是正当用途），`bshift`、`bfill` 和 `labels.rs` 的每个函数都在文档里标明自己读未来；挡住它的是下面这些在边界上的机制。

## 哪些函数读未来

| 函数 | 读多远 | 详见 |
|---|---|---|
| `bshift`、`bfill`（`barq-generic/src/elementwise.rs`） | `bshift` 拉回 `n` 根之后的值；`bfill` 用下一个已知值填缺口 | [barq-generic](../crates/barq-generic.md) |
| `fixed_horizon_returns`、`trend_labels`、`triple_barrier`、`pivot_labels` | `h` 根、`h` 根、≤ `max_hold`、无界 | [机器学习标签](../algorithms/ml-labels.md) |
| `forward_paths`（`barq-analysis/src/events.rs`） | 每个事件之后 `after` 根 | [事件研究](../algorithms/event-study.md) |
| 因子表达式里偏移为负的 `Ref`，如 `Ref($close, -1)` | 偏移的绝对值 | [barq-expr](../crates/barq-expr.md) |

`bshift`、`bfill`、整个 `labels.rs` 读未来；`pivot_labels` 的前视无界；即使因果因子变信号也要延迟一根。

## 哪些机制挡住它

| 机制 | 层 | 做什么 | 详见 |
|---|---|---|---|
| `available_time` | 数据 | 一切都用 `available_time`：过滤、面板行轴、对齐。`event_time` 只校验类型和非空，从不读取 | [按时点对齐 align](../algorithms/pit-alignment.md) |
| `uses_future` | 因子 | `Ref` 的偏移是负数字面量时为真（`parse.rs:37-51`）；`barq.factors` 与研究框架的 `evaluate` 见到它直接报错 | [barq-expr](../crates/barq-expr.md) |
| `signal_delay` → `_delay` | Python 策略 | 默认 1：`delayed[bars:] = array[:-bars]`，顶部补 False | [@barq.strategy 的参数扫描](../algorithms/strategy-sweep.md) |
| `delay` | Rust 门面 | `rule(factor[row − delay, symbol])`；前 `delay` 行不产生信号 | [SignalSweep：因子到信号列](../algorithms/signal-sweep.md) |
| `targets` 的平移 | 研究 | `out[1:] = orders[:-1]`：`t` 决定，`t+1` 执行 | [评估框架与合同检查](../algorithms/research-harness.md) |
| `contract_check` | 研究 | 每个表达式在 800 根合成面板的全长和前 600 行上各算一次，`!allclose(前缀结果, 全长结果[:600])`——加了后面的 K 线会改变前面的值 = 前视 | [评估框架与合同检查](../algorithms/research-harness.md) |
| `not_suspicious` | 研究 | t 统计量 > `suspicious_t`（8）视为前视，门槛不通过 | [门槛、多重比较与原创性检查](../algorithms/research-gates.md) |

## 相关

- [barq-signals：信号工具箱](../crates/barq-signals.md)：所属 crate；`labels.rs` 是其中唯一读未来的模块。
- [机器学习标签](../algorithms/ml-labels.md)：四个读未来的标签函数，各自看多远。
- [按时点对齐 align](../algorithms/pit-alignment.md)：数据层用发布时间而不是事件时间，从源头不泄露未来。
- [@barq.strategy 的参数扫描](../algorithms/strategy-sweep.md)：`signal_delay` 默认 1 的完整扫描流程。
- [SignalSweep：因子到信号列](../algorithms/signal-sweep.md)：Rust 侧把延迟作为参数的因子扫描。
- [评估框架与合同检查](../algorithms/research-harness.md)：前缀一致性检查与 `t+1` 执行。
