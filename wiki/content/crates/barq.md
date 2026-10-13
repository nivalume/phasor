---
title: barq：门面 crate
kind: crate
summary: 把工作区的 crate 以短名重新导出、提供 prelude，并用 SignalSweep 把一个因子扫成参数 × 标的的信号列。
depends: [barq-analysis, barq-core, barq-data, barq-exec, barq-expr, barq-generic, barq-indicators, barq-model, barq-records, barq-signals, barq-sim]
related: [signal-sweep, barq-py, column-labels, barq-expr, barq-sim, barq-analysis]
tags: [facade, prelude, sweep]
sources:
  - crates/barq/Cargo.toml
  - crates/barq/src/lib.rs
  - crates/barq/src/sweep.rs
  - crates/barq/examples/factor_sweep.rs
  - crates/barq/tests/sweep.rs
  - crates/barq/tests/factor_sweep.rs
  - crates/barq/tests/factor_path.rs
order: 120
---

# barq：门面 crate

`barq` 是 Rust 用户的入口：它自己几乎不计算，只把工作区里的 crate 重新导出、给出一份 `prelude`，再补上一个 [`SignalSweep`](../algorithms/signal-sweep.md)。Python 用户走的是 [barq-py](../crates/barq-py.md)，那一层直接依赖各成员 crate，不经过这个门面。

## 定位

- **`barq`**（Rust 门面，364 行）：把所有 crate 以短名重新导出（`barq::sim`、`barq::expr`…），提供 `prelude`，以及 `SignalSweep`——"参数和标的共用一条[列轴](../concepts/column-axis.md)"的最小完整表达。

`lib.rs` 只有三样东西：

- **短名模块**（`lib.rs:31-44`）：`analysis`、`core`、`exec`、`expr`、`generic`、`indicators`、`model`、`records`、`signals`、`sim` 各对应一个成员 crate；`data` 只在开启 `data` 特性时导出；`sweep` 是本 crate 自己的模块。
- **`prelude`**（`lib.rs:46-71`）：一次 `use barq::prelude::*;` 拿到写一次扫描所需的名字——[列标签](../concepts/column-labels.md) `Columns`/`Label`/`Level`、[`FlexArray`](../concepts/flex-array.md)、`Matrix`、`Error`/`Result`；内核的 `Order`、`OrderResult`、`ExecuteOrderState`、`ProcessOrderState`；`barq_model::enums::*` 与 `InstrumentId`/`Symbol`/`Venue`、`Price`/`Quantity`；因子引擎的 `FactorSet`、`FactorInputs`、`FactorValues`、`ComputeBackend`、`NativeBackend`、`alpha158`、`alpha360`；指标的 `ma`、`MaParams`、`combinations`；信号的 `clean_enex`、`exit_after`、`partition_ranges`、`ExitWindow`、`FirstSignal`；模拟的 `FromSignals`、`simulate_from_signals`、`simulate_from_signals_parallel`、`Groups`、`CallSeq`；记录的 `OrderRecords`、`exit_trades`；分析的 `Portfolio`、`Scoring`、`Reduce`、`best_column`、`group_by`、`Select`、`Tag`、`Value`、`sharpe_ratio`、`total_return`、`DAILY`；以及 `SignalSweep`。
- **特性开关**（`crates/barq/Cargo.toml`）：默认不开任何特性。`data` 打开可选依赖 [barq-data](../crates/barq-data.md) 并导出 `barq::data`；`polars` 打开 `barq-expr/polars`，`prelude` 里多出 `PolarsBackend`（见 [Polars 后端](../concepts/polars-backend.md)）。

## 文件与阅读顺序

| 顺序 | 文件 | 行数 | 读什么 | 第一遍可跳过 |
|---|---|---:|---|---|
| 2 | `barq/src/lib.rs` | 71 | crate 地图与 `prelude` | — |
| 3 | `barq/src/sweep.rs` | 293 | `SignalSweep::from_factor` 42–123：`out_col = case_idx × n_symbols + symbol`；标签 107–113；`args` 170–186 | `expand` |

顺序号沿用 Python 层的整体阅读顺序（完整表在 [barq-py](../crates/barq-py.md#文件与阅读顺序)）。示例与集成测试：

| 文件 | 内容 |
|---|---|
| `crates/barq/examples/factor_sweep.rs` | 一个因子求值一次，按两组规则 × 三个标的扫成六列，多线程 `Portfolio::from_signals`，按列标签打印总收益；`cargo run -p barq --example factor_sweep` |
| `crates/barq/tests/factor_sweep.rs` | `SignalSweep` 扫出的每一列与逐个单独运行一致、串行与 4 线程一致；连续为真的进场条件在止损后能重新进场 |
| `crates/barq/tests/sweep.rs` | 均线交叉扫描（直接用 `ma` 与 `crossed_above_1d`，不经 `SignalSweep`）的结构性断言：一次运行覆盖 `参数 × 标的` 列、多线程与串行逐位一致、标签能从列追回参数、从未交易的列以 `+inf` 夏普排在朴素排名最前 |
| `crates/barq/tests/factor_path.rs` | 因子表达式 → 手工延迟一根的信号 → `clean_enex` → `Portfolio::from_signals`，端到端走通 |

## 核心算法与概念

- [SignalSweep：因子到信号列](../algorithms/signal-sweep.md)：对每个 case × 标的把延迟后的因子值交给 `rule`，列号 `case_idx × n_symbols + symbol`，信号不清洗。

## 调用链

`crates/barq/examples/factor_sweep.rs` 是一条完整的 Rust 路径：

```text
FactorSet::compile([(name, 表达式)]) → evaluate(&FactorInputs::from_ohlcv(&panel)) → values.get(name)   ← barq-expr
SignalSweep::from_factor((name, factor), &symbols, &cases, delay, rule)                                 ← 本 crate
sweep.expand(&close) → swept_close（标的列按 case 重复）
FromSignals { init_cash, fees, ..sweep.args(&swept_close)? }                                            ← barq-sim 的参数结构
Portfolio::from_signals(&args, threads) → total_return() …                                              ← barq-analysis
sweep.columns().describe(col)：把每列结果追回 factor / case / symbol
```

因子求值见 [barq-expr](../crates/barq-expr.md)，模拟见 [barq-sim](../crates/barq-sim.md)，从记录到收益见 [barq-analysis](../crates/barq-analysis.md)。
