---
title: 前缀和与 EWM 窗口核
kind: algorithm
summary: 滚动均值和标准差用前缀和，极值用单调队列，EWM 每根衰减且故意不融合乘加，都是为了与参考实现逐位一致。
parent: barq-generic
related: [monotonic-queue, rolling-accumulators, float-tolerance, parity-testing, indicator-sweep]
tags: [rolling, ewm, nan, parity]
sources:
  - crates/barq-generic/src/rolling.rs:71-464
  - crates/barq-generic/tests/parity_generic.rs
order: 301
---

# 前缀和与 EWM 窗口核

[barq-generic](../crates/barq-generic.md) 的 `rolling.rs` 里，每个窗口函数都选了参考实现的写法而不是数值上"更好"的写法：前缀和相减而不是重新求和、展开式方差而不是 Welford、乘加不融合。目的都是与 vectorbt 逐位一致，由 [差分测试](../tooling/parity-testing.md) 钉住。

## 位置

- `crates/barq-generic/src/rolling.rs:1-45`（模块文档与 `resolve_minp`）
- `crates/barq-generic/src/rolling.rs:71-99`（`rolling_apply_1d`）
- `crates/barq-generic/src/rolling.rs:107-141`（`rolling_mean_1d`）
- `crates/barq-generic/src/rolling.rs:151-201`（`rolling_std_1d`）
- `crates/barq-generic/src/rolling.rs:221-258`（`rolling_extremum_1d`）
- `crates/barq-generic/src/rolling.rs:268-310`（`expanding_*`）
- `crates/barq-generic/src/rolling.rs:329-377`（`ewm_mean_1d`）
- `crates/barq-generic/src/rolling.rs:395-464`（`ewm_std_1d`）
- `crates/barq-generic/src/rolling.rs:607-675`（测试：相等守卫和故意不融合的乘加）

## `barq-generic` 的窗口核（`rolling.rs`）

| 函数 | 算法 | 要点 |
|---|---|---|
| `rolling_mean_1d` 107–141 | 单趟前缀和：存每个下标的 `cumsum`（只加非 NaN）和 `nancnt`；`i ≥ window` 时窗口和 = `cumsum − cumsum_arr[i−window]`，有效长度 = `window − (nancnt − nancnt_arr[i−window])` | 均值只对有效根算；`< minp` 给 NaN。**不是**数值上更好的写法——是参考实现的写法，`wide_scale` fixture（1e8 旁边放 1e-8）能把"重新求和"和"相减"区分开 |
| `rolling_std_1d` 151–201 | 同样的前缀和加 `cumsum_sq`；方差用展开式 `E[x²] − 2·mean·Σx + len·mean²`，套 `.abs()` 防常量序列抵消成负数 | `ddof` 是调用方参数（0 总体，1 样本）；`window_len == ddof` 也给 NaN。不是 Welford，为了对照 |
| `rolling_extremum_1d` 221–258 | 单调队列（[与因子引擎同款](../algorithms/monotonic-queue.md)）；相等时用 `>=`/`<=` 弹出，即平局被更新的 K 线替代 | 全 NaN 窗口返回当前 NaN 而不是 ±inf；比较不是累加，所以与重扫逐位一致 |
| `expanding_*` 268–310 | `rolling_*(a, len, minp)` | `minp > len` 是 `Err`，参考实现如此，"不明显正确但避免无声分歧" |
| `ewm_mean_1d` 329–377 | 只有 `span` 参数：`alpha = 2/(span+1)`；`old_wt *= (1−α)` **每根都衰减，包括 NaN 根**（pandas `ignore_na=False`）；`adjust=true` 时 `old_wt += new_wt`，否则 `old_wt = 1` | 两个数值细节：相等守卫 `weighted_avg != cur` 不是优化——重算不变的值会引入 ~2e-15 漂移（测试里 span 10 的常量 8.19478 就漂）；`old_wt·avg + new_wt·cur` **故意不用 `mul_add`**，融合只舍入一次而参考舍入两次，差 1 ulp |
| `ewm_std_1d` 395–464 | 跟踪 `mean`、加权方差 `cov`、`sum_wt`、`sum_wt2`；输出 `sqrt(sum_wt²/(sum_wt² − sum_wt2) · cov)`（pandas `bias=False`） | 第 0 根永远 NaN；参考实现收一个从不读的 `ddof`，这里去掉了 |
| `rolling_apply_1d` 71–99 | 通用逃生口：用前缀 NaN 计数判断 `valid ≥ minp`，然后把**含 NaN 的整个窗口**交给闭包 | O(n·w·cost)；[`barq-analysis`](../crates/barq-analysis.md) 的滚动指标全走它 |

## 相关

- [barq-generic：通用窗口算子](../crates/barq-generic.md)：所属 crate，三条语义约定与 `min_periods`、NaN 的阅读陷阱。
- [单调队列：窗口极值与极值位置](../algorithms/monotonic-queue.md)：`rolling_extremum_1d` 用的同一个算法，在因子引擎里还算极值位置。
- [O(1) 滚动累加器](../algorithms/rolling-accumulators.md)：因子引擎的另一套滚动核（`window.rs`），用平移求和与周期重建控制漂移；`Mean` 则直接调这里的 `rolling_mean_1d`。
- [浮点容差与零钉](../concepts/float-tolerance.md)：为什么 `weighted_avg != cur` 是带 `#[allow]` 的精确比较而不是容差。
- [指标参数网格与共享缓存](../algorithms/indicator-sweep.md)：MA、MSTD、BBANDS、RSI 都直接调用这里的 `rolling_*`/`ewm_*`。
- [差分测试与 fixture](../tooling/parity-testing.md)：`wide_scale` 等 fixture 如何把写法差异变成可见的失败。
