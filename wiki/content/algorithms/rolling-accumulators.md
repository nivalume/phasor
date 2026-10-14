---
title: O(1) 滚动累加器
kind: algorithm
summary: slide_into 每行 push 进入值、pop 离开值、每 window 行重建一次；Moments 平移求和并精确识别常量窗口，总代价 O(n)。
parent: phasor-expr
related: [native-evaluation, order-statistics, monotonic-queue, generic-windows, qlib-operators]
tags: [rolling, numerics, accumulators]
sources:
  - crates/phasor-expr/src/window.rs
  - crates/phasor-expr/src/eval.rs:963-1046
order: 704
---

# O(1) 滚动累加器

滚动窗口统计如果每行重扫窗口是 O(n·w)；`window.rs` 的累加器每行只做一次进入、一次离开，把代价降到 O(1)/行，同时用周期重建、平移求和与常量检测把浮点误差控制在可验证的范围内。

## 位置

- `crates/phasor-expr/src/window.rs:1-22`：模块文档
- `crates/phasor-expr/src/window.rs:49-70`：驱动 `slide_into`
- `crates/phasor-expr/src/window.rs:75-163`：`Moments`
- `crates/phasor-expr/src/window.rs`：`Sum`/`Count` → `Deviation` → `Wma` → `Regression` → `Pair` → `Shape`
- `crates/phasor-expr/src/eval.rs:963-1046`：精度测试 `incremental_windows_match_a_full_rescan`

![窗口核](../../../docs/img/zh/window.svg)

## 驱动 slide_into

**驱动 `slide_into`**（49–70）：每行 `start = (row+1) − window`；`row == 0` 或 `row − rebuilt ≥ window` 时 `rebuild(window切片)`，否则 `push(新行)`、`pop(离开行)`；再 `value()`。**每 `window` 行重建一次**：一次 O(window)，摊到 `window` 行 → 总 O(n)；加减带来的舍入漂移不会累积超过约两个窗口。`Window::All` 只在第 0 行重建一次。

## Moments：平移求和与常量检测

**`Moments`**（75–163）是共用的"平移求和"状态：

```text
shift = 空窗口推入的第一个非 NaN 值（每次 rebuild、或 pop 把窗口掏空后都重新锚定）
d = x − shift；sum += d；sum_sq += d²；pop 对称减
run = 最近连续等于 last 的推入次数；constant() = run ≥ count       ← pop 不动 run 仍正确：窗口里就是最近 count 次推入
centered_sq() = Σ(x − mean)² = sum_sq − sum²/count，钳 ≥ 0，constant 时精确 0
std(ddof) = sqrt(centered_sq / (count − ddof))，count ≤ ddof 时 NaN
NaN：push/pop 都忽略——占一个窗口位，但不进任何和
```

为什么要 `shift`：100 附近价格的方差，否则是两个大平方的小差（灾难性抵消）。为什么要常量检测：Qlib 在 `Corr` 里测 `va == 0`、`Rsquare` 里测 `yvar <= 0`，是**精确**比较，运行和在常量序列上会留下残差而不是 0。

## 各累加器

| 累加器 | 状态 | `value()` |
|---|---|---|
| `Sum` | count, sum | sum；全 NaN → NaN |
| `Count` | count | count；0 → NaN |
| `Deviation`（Std/Var） | `Moments` | `std(1)`；`Var = std²`（继承 sqrt 的舍入） |
| `Wma` | origin, count, sum, `weighted = Σ(row − origin)·x` | 权重 `1..=len` 归一化（含 NaN 位），`(weighted − lag·sum)/weight_sum/count`，`lag = (start − origin) − 1` |
| `Regression`（Slope/Rsquare/Resi） | origin，y: `Moments`，`sx sxx`（整数，精确），`sxy = Σx(y − shift)` | 换到 1-based 窗口位置：`sx' = sx − lag·n`，`sxx' = sxx − 2·lag·sx + lag²·n`，`sxy' = sxy − lag·sy'`；`xvar = n·sxx' − sx'²`；`cov = n·sxy' − sx'·sy'`（y 常量时精确 0）；`slope = cov/xvar`；`Rsquare = cov²/(xvar·yvar)`，`sample_std ≤ 2e-5` 时 NaN（Qlib 的掩码）；`Resi = (last − shift) − (slope·len + intercept)` |
| `Pair`（Corr/Cov） | 四个 `Moments`（两边各自的全部非 NaN；两边配对行）+ `cross = Σ(a − shift_a)(b − shift_b)` | `Corr` 先看任一边**自身**的 `std(1) ≤ 2e-5` → NaN；`cov = cross − n·mean_a·mean_b`（任一边常量时精确 0）；`Cov = cov/(n−1)`；`Corr = cov/√(va·vb)` |
| `Shape`（Skew/Kurt） | `Moments` + `Σd³`、`Σd⁴` | `m3 = s3 − 3μs2 + 2nμ³`，`Skew = n√(n−1)/(n−2) · m3/m2^1.5`（pandas G1）；`m4 = s4 − 4μs3 + 6μ²s2 − 3nμ⁴`，`Kurt = (n−1)/((n−2)(n−3)) · ((n+1)(n·m4/m2² − 3) + 6)`（pandas G2）；常量窗口 NaN |

## Mean、窗口规则与精度

`Mean` 在 `phasor_generic::rolling::rolling_mean_1d`：前缀和 + 前缀 NaN 计数，O(n)，不做周期重建。
`min_periods = 1` 处处如此；`N = 0` 是扩展窗口；全 NaN 窗口返回 NaN。
精度钉住：`incremental_windows_match_a_full_rescan`（`eval.rs:963-1046`）在 6 条压力序列（NaN 空洞、全 NaN 段、常量段、1e4 水平上的 1e-3 波动、阶跃）× 窗口 `{1,2,3,5,20,61,299,300,1000}` 上以 1e-9 相对误差对照朴素重扫。

`Max`/`Min` 不是累加器（没有减法能把一个值从最大值里移除），它们走[单调队列](../algorithms/monotonic-queue.md)。

## 相关

- [phasor-expr：因子表达式引擎](../crates/phasor-expr.md)：所属 crate；阅读陷阱第 5、10、12 条（`min_periods = 1`、增量核与重扫的 1e-9、WMA 权重）都与这里有关。
- [原生求值：按标的并行走图](../algorithms/native-evaluation.md)：`rolling_into` 把滚动节点分派到这些核。
- [顺序统计量：有序环与树状数组](../algorithms/order-statistics.md)：需要有序窗口的 `Quantile`/`Med`/`Rank`/`Mad` 走另一套结构。
- [单调队列：窗口极值与极值位置](../algorithms/monotonic-queue.md)：`Max`/`Min`/`IdxMax`/`IdxMin`。
- [前缀和与 EWM 窗口核](../algorithms/generic-windows.md)：`Mean` 用的 `phasor_generic` 前缀和核。
- [因子算子与 Qlib 对照](../reference/qlib-operators.md)：每个累加器对应的 Qlib 算子与对照精度。
