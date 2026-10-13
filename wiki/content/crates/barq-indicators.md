---
title: barq-indicators：技术指标
kind: crate
summary: MA、MSTD、BBANDS、RSI、STOCH、MACD、ATR、OBV 在显式展开的参数网格上计算，共享中间量用缓存只算一次。
depends: [barq-core, barq-generic]
related: [barq-generic, column-axis, column-labels, barq, generic-windows]
tags: [indicators, sweep, cache]
sources:
  - crates/barq-indicators/src/lib.rs
  - crates/barq-indicators/src/sweep.rs
  - crates/barq-indicators/src/indicators.rs
  - crates/barq-indicators/tests/parity_indicators.rs
order: 50
---

# barq-indicators：技术指标

`barq-indicators` 把 [barq-generic](../crates/barq-generic.md) 的窗口算子组合成有名字的技术指标，并在调用方展开好的参数组合上一次算完整个网格。列布局与全仓库的[列轴](../concepts/column-axis.md)同一个公式；只有 Rust 接口，经 [barq](../crates/barq.md) 门面导出。

## 定位

三个小 crate，都是"数组进、数组出"的纯函数库，没有状态、没有并行：

| crate | 做什么 | 被谁用 |
|---|---|---|
| `barq-indicators` | MA、MSTD、BBANDS、RSI、STOCH、MACD、ATR、OBV 与参数网格 | [`barq`](../crates/barq.md) 门面、测试；Python 未接（`plan.md` H1） |

另外两个是 [barq-generic](../crates/barq-generic.md) 和 [barq-signals](../crates/barq-signals.md)，它们的定位写在各自的页面上；三条贯穿的语义约定（`min_periods`、NaN、前视）见 [barq-generic](../crates/barq-generic.md#定位)。

## 文件与阅读顺序

| 顺序 | 文件 | 行数 | 读什么 | 第一遍可跳过 |
|---|---|---:|---|---|
| 4 | `barq-indicators/src/sweep.rs` | 242 | 全部：`col = comb × n_input + input_col` 的契约、`combinations`、`sweep_columns` | — |
| 5 | `barq-indicators/src/indicators.rs` | 902 | `Indicator`/`Cache`/`per_column`/`place` 29–133；`ma` 149–169 是模板；再 `bbands` 237–292、`rsi` 306–353、`macd` 451–511 看缓存键共享 | `true_range`/`obv` 最后 |

顺序编号是三个小 crate 共用的一条路线：1–3 在 [barq-generic](../crates/barq-generic.md#文件与阅读顺序)，6–11 在 [barq-signals](../crates/barq-signals.md#文件与阅读顺序)。

输入输出约定：`barq-indicators` 收 `&Matrix` + 已展开的参数组合切片 + 可选标签，回 `Indicator`。

## 类图

`rolling` 与 `elementwise` 是 [barq-generic](../crates/barq-generic.md#类图) 的模块，成员画在那张图里。

```mermaid
classDiagram
  direction LR
  class Matrix
  class Indicator { +Columns columns; -Vec~(&str, Matrix)~ outputs; +get(name) names() }
  class Cache~K~ { -HashMap~K, Matrix~ entries; +get_or_insert(key, compute) }
  class ParamLevel {
    -String name; -Vec~Label~ values（每个组合一个）
    +new()$
  }
  class MaParams { +usize window; +bool ewm }
  class BbParams { +window; +ewm; +f64 alpha }
  class StochParams { +k_window; +d_window; +d_ewm }
  class MacdParams { +fast_window; +slow_window; +signal_window; +macd_ewm; +signal_ewm }
  class sweep {
    <<mod>>
    +combinations(sizes) +sweep_columns(params, input) +column_of(comb, input_col, n_input)
  }
  class indicators {
    <<mod>>
    +ma() mstd() bbands() rsi() stoch() macd() true_range() atr() obv()
  }
  class rolling {
    <<mod>>
  }
  class elementwise {
    <<mod>>
  }

  indicators ..> rolling
  indicators ..> elementwise
  indicators ..> Cache
  indicators ..> sweep
  indicators ..> Indicator
  sweep ..> ParamLevel
  Indicator --> Matrix
```

## 核心算法与概念

- [指标参数网格与共享缓存](../algorithms/indicator-sweep.md)：`column_of = comb × n_input + input_col` 的列布局契约、按 `(window, ewm)` 等键共享中间量的 `Cache`，以及每个指标的公式。

## 阅读陷阱

1. **`ddof`**：`rolling_std`/`mstd`/`bbands` 显式传，vectorbt 默认 0，pandas 默认 1；`ewm_std` 完全不看 ddof。
2. **RSI 不是 Wilder 的**；`rs = +inf` 是活值，用 parity 测试里的 `agree` 而不是 `is_close` 比较。

`min_periods` 因调用方而异（指标硬编码 `window`，`barq-expr` 传 `Some(1)`）和 NaN 让窗口缩小这两条，见 [barq-generic](../crates/barq-generic.md#阅读陷阱)。

## 与参考实现的对应

| barq | vectorbt | 差异 |
|---|---|---|
| `ma mstd bbands rsi stoch macd atr obv` | `indicators/nb.py` 的 `*_nb`、`*_cache_nb`/`*_apply_nb`；`vbt.MA.run` 等 | 扫描用显式组合而不是 `IndicatorFactory` 的 `param_product`；标签用 `Columns` 而不是 `MultiIndex` |

## 延伸资料

- J. Welles Wilder, *New Concepts in Technical Trading Systems*（1978）：RSI、TR/ATR 的原始定义，对照这里的 Cutler/EMA 变体；John Bollinger, *Bollinger on Bollinger Bands*（2001）；Gerald Appel 的 MACD；Joseph Granville 的 OBV。
- vectorbt 文档 `IndicatorFactory`。
- 更多见 [延伸资料](../reference/further-reading.md#通用算子指标信号)。

## 自测

1. `combinations(&[3, 2])` 的顺序是什么？`column_of(4, 1, 2)` 是哪一列？它对应哪组参数？
