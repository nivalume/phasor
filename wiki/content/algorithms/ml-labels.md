---
title: 机器学习标签
kind: algorithm
summary: 四个监督学习目标，每个都读未来；triple_barrier 返回决定行，给 purge 切分用。只能做 y，绝不能做信号。
parent: barq-signals
related: [look-ahead, ts-cross-validation, event-study, research-harness]
tags: [labels, look-ahead, ml]
sources:
  - crates/barq-signals/src/labels.rs
order: 404
---

# 机器学习标签

`labels.rs` 是 [barq-signals](../crates/barq-signals.md) 里唯一读未来的模块：第 `t` 根的标签由 `t` 之后的 K 线算出——这正是标签的意义——所以它们只能做模型要预测的 `y`，不能做进出场或特征（模块文档 1–14 行）。每个函数的文档都有一段 "Look-ahead" 写明看多远，训练/测试切分要留的空隙（purge）可以直接从签名读出来；规则本身见 [延迟与前视偏差](../concepts/look-ahead.md)。

## 位置

- `crates/barq-signals/src/labels.rs:1-23`（模块文档）
- `crates/barq-signals/src/labels.rs:41-48`（`fixed_horizon_returns`）
- `crates/barq-signals/src/labels.rs:83-89`（`trend_labels`）
- `crates/barq-signals/src/labels.rs:138-146`（`triple_barrier`）
- `crates/barq-signals/src/labels.rs:224-234`（`pivot_labels`）

## 四个标签函数

**`labels.rs`**（全部读未来，只能做 `y`）：

| 函数 | 定义 | 前视 |
|---|---|---|
| `fixed_horizon_returns(h)` | `close[t+h]/close[t] − 1`，尾部 NaN | `h` 根 |
| `trend_labels(h, θ)` | 上式 `> θ` → +1，`< −θ` → −1，否则 0；**未定义也是 0** | `h` 根 |
| `triple_barrier(up, down, max_hold)` | 固定百分比障碍（不按波动率缩放）：从 `t+1` 走到 `min(t+max_hold, 末尾)`，先碰 `r ≥ up` → +1、先碰 `r ≤ −down` → −1、都没碰 → 0；返回 `(label, 决定行)`，决定行是 purge 切分要用的东西；只看收盘价，不看盘中高低 | ≤ `max_hold` |
| `pivot_labels(θ)` | 收盘价上的 zig-zag：`Leg::{Unknown, Up{peak}, Down{trough}}` 状态机，反向 θ 确认前一个候选为峰（−1）/谷（+1）；最后一段的候选永不确认 | **无界** |

## 相关

- [barq-signals：信号工具箱](../crates/barq-signals.md)：所属 crate；"标签里 0 一词多义"是它的阅读陷阱之一。
- [延迟与前视偏差](../concepts/look-ahead.md)：哪些函数读未来、哪些机制挡住它；这里是读未来的一端。
- [时间序列交叉验证](../algorithms/ts-cross-validation.md)：`purged_kfold` 的 `purge` 就是按 `triple_barrier` 的决定行定的。
- [事件研究](../algorithms/event-study.md)：`forward_paths` 同样读未来，同样只做分析。
- [评估框架与合同检查](../algorithms/research-harness.md)：研究层的 leak 负对照 `close[t+25]/close[t+1] − 1` 就是一个前向收益，被故意当成因子，必须被拒绝。
