---
title: 指标参数网格与共享缓存
kind: algorithm
summary: 调用方先展开参数组合，输出列是 comb × n_input + input_col；共享中间量按决定它的参数做键，扫描里只算一次。
parent: phasor-indicators
related: [column-axis, column-labels, generic-windows, signal-sweep, strategy-sweep]
tags: [indicators, sweep, cache, columns]
sources:
  - crates/phasor-indicators/src/sweep.rs
  - crates/phasor-indicators/src/indicators.rs:29-602
order: 501
---

# 指标参数网格与共享缓存

[phasor-indicators](../crates/phasor-indicators.md) 的每个指标函数都收一组已经展开好的参数组合，把所有组合的结果写进同一个矩阵；列的排法与全仓库的[列轴](../concepts/column-axis.md)一致，标签用 [`Columns` 与 `Level`](../concepts/column-labels.md) 生成。

## 位置

- `crates/phasor-indicators/src/sweep.rs:44`（`combinations`）
- `crates/phasor-indicators/src/sweep.rs:103`（`sweep_columns`）
- `crates/phasor-indicators/src/sweep.rs:135-137`（`column_of`）
- `crates/phasor-indicators/src/indicators.rs:29-133`（`Indicator`、`Cache`、`per_column`、`place`）
- `crates/phasor-indicators/src/indicators.rs:149-169`（`ma`，其余指标的模板）
- `crates/phasor-indicators/src/indicators.rs:237-292`（`bbands`）、`306-353`（`rsi`）、`451-511`（`macd`）

## 列布局契约

**列布局契约**（`sweep.rs`）：指标函数不做笛卡尔积，调用方先用 `combinations(&[sizes])` 展开（最后一级变化最快）再映射成参数结构体；组合 `i`、输入列 `s` 的输出列是 `column_of(i, s, n_input) = i × n_input + s`。`sweep_columns` 用 `Level::repeat(param, values, n_input)` 和 `Level::tile("symbol", labels, n_combs)` 生成标签——就是[列轴与列主序](../concepts/column-axis.md)里的公式。

## 共享中间量缓存

**共享中间量缓存**：`Cache<K> = HashMap<K, Matrix>`。`ma`/`mstd`/`atr` 键 `(window, ewm)`；`bbands` 两个缓存都键 `(window, ewm)`，`alpha` 不进键，扫 `alpha` 只付带状算术；`rsi` 缓存成品；`stoch` 只按 `k_window` 缓存 `%K`；`macd` **一个**缓存同时服务快慢窗口，某窗口在一行当快、另一行当慢只算一次。每次用都 `.clone()` 出来再 `place`（借用冲突），多一份 `n_rows × n_input` 拷贝。

## 公式

公式（`moving_average` 的 `minp` 永远等于 `window`）：

| 指标 | 公式 | 注意 |
|---|---|---|
| MA | `ewm ? ewm_mean_1d(x, w, w, adjust) : rolling_mean_1d(x, w, Some(w))` | |
| MSTD | 同上的 std，`ddof` 共享 | |
| BBANDS | `middle = MA`，`upper/lower = middle ± alpha × MSTD` | `alpha` 是 `Label::Float` |
| RSI | `delta = diff(close,1)`；`up = max(δ,0)`、`down = max(−δ,0)`（写法让 NaN 两边都保持 NaN）；`rs = MA(up)/MA(down)`；`rsi = 100 − 100/(1+rs)` | **不是 Wilder 的 RSI**：`ewm=true` 是 span 平滑 α = 2/(w+1)，`ewm=false` 是简单平均（Cutler）；无下跌 → `rs = +inf` → RSI 恰好 100"经由无穷而不是特判" |
| STOCH | `%K = 100 × (close − min(low,k)) / (max(high,k) − min(low,k))`，`%D = MA(%K, d)` | 平盘区间 0/0 = NaN，fixture 钉住 |
| MACD | `fast − slow`，`signal = MA(macd, signal_window)` | |
| TR / ATR | `row 0: h − l`；否则 `max(h−l, \|h−c_prev\|, \|l−c_prev\|)`（`f64::max` 忽略 NaN）；`atr = MA(tr)` | `tr` 整个扫描只算一次 |
| OBV | `signed = close < prev ? −v : +v`；`nancumsum` | 第 0 根与 NaN 比较为假 → 算正 |

MA、MSTD 和 EWM 背后的窗口核见 [前缀和与 EWM 窗口核](../algorithms/generic-windows.md)。

## 相关

- [phasor-indicators：技术指标](../crates/phasor-indicators.md)：所属 crate，`ddof` 与 RSI 的阅读陷阱。
- [列轴与列主序](../concepts/column-axis.md)：`param_idx × n_symbols + symbol_idx` 这条公式在全仓库的出处。
- [列标签：Columns、Level 与 group_indices](../concepts/column-labels.md)：`Level::repeat`/`tile` 如何生成扫描的列标签。
- [前缀和与 EWM 窗口核](../algorithms/generic-windows.md)：`rolling_mean_1d`、`rolling_std_1d`、`ewm_mean_1d` 的数值细节。
- [SignalSweep：因子到信号列](../algorithms/signal-sweep.md)：同一个"参数重复、标的平铺"的列布局，用在因子到信号的扫描上。
