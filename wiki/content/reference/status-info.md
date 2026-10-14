---
title: 拒单原因 StatusInfo
kind: reference
summary: 14 种 StatusInfo 各由撮合内核的哪一步、哪几行触发，以及对应的状态是 Ignored 还是 Rejected。
related: [errors-vs-rejections, execute-order-pipeline, buy-leg, sell-leg, phasor-model, phasor-exec]
tags: [rejections, orders]
sources:
  - crates/phasor-exec/src/kernel.rs
  - crates/phasor-model/src/enums.rs
order: 10
---

# 拒单原因 StatusInfo

订单没有成交时，[phasor-exec](../crates/phasor-exec.md) 返回的 `OrderResult` 里 `status` 是 `Ignored` 或 `Rejected`，`status_info` 说明原因；这不是错误（见[错误与拒单](../concepts/errors-vs-rejections.md)）。下表按原因列出它在撮合内核里的触发位置。

## 14 种 `StatusInfo` 的触发点

| `StatusInfo` | 状态 | 触发位置 |
|---|---|---|
| `SizeNaN` / `PriceNaN` | Ignored | `kernel.rs:585-596`，校验之前 |
| `ValueNaN` / `ValueZeroNeg` | Ignored / Rejected | TargetPercent 阶段 `kernel.rs:620-635` |
| `ValPriceNaN` | Ignored | Value/TargetValue 阶段 `kernel.rs:643-648` |
| `RandomEvent` | Rejected | 分发前 `kernel.rs:686-692` |
| `NoCashLong` | Rejected | buy，`cash_limit == 0` `kernel.rs:94-98` |
| `NoOpenPosition` | Rejected | buy（ShortOnly 无空头可平）`kernel.rs:105-110`；sell（LongOnly 无多头可卖）`kernel.rs:309-314` |
| `SizeZero` | Ignored | buy `kernel.rs:119-124, 172-177`；sell `kernel.rs:320-325` |
| `MaxSizeExceeded` | Rejected | buy `kernel.rs:126-134`；sell `kernel.rs:293-301` |
| `CantCoverFees` | Rejected | buy 反解 ≤ 0 `kernel.rs:151-156`；sell 两处 `kernel.rs:259-264, 342-347` |
| `MinSizeNotReached` | Rejected | buy `kernel.rs:178-183`；sell `kernel.rs:326-331` |
| `PartialFill` | Rejected | buy `kernel.rs:185-190`；sell `kernel.rs:332-337` |
| `NoCashShort` | Rejected | sell `kernel.rs:245-252` |

枚举定义和判别值在 `crates/phasor-model/src/enums.rs`，记录里存的是整数，所以判别值被测试钉死。

## 相关

- [错误与拒单](../concepts/errors-vs-rejections.md)：拒单是正常结果，只有不变量被破坏才是 `Err`。
- [execute_order 的固定顺序](../algorithms/execute-order-pipeline.md)：`SizeNaN`、`PriceNaN`、`RandomEvent` 在流程里的位置。
- [数量换算链与 percent](../algorithms/size-cascade.md)：`ValueNaN`、`ValueZeroNeg`、`ValPriceNaN` 来自换算链。
- [buy：买入、平空与 lock_cash 预算](../algorithms/buy-leg.md)：`buy` 里的各个拒单出口。
- [sell：卖出与开空](../algorithms/sell-leg.md)：`sell` 里的各个拒单出口。
- [phasor-model：领域模型](../crates/phasor-model.md)：`StatusInfo` 的定义和被测试钉死的判别值。
