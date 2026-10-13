---
title: SignalSweep：因子到信号列
kind: algorithm
summary: 对每个 case × 标的把延迟后的因子值交给 rule 生成进出场，列号 case_idx × n_symbols + symbol，信号不清洗。
parent: barq
related: [column-labels, strategy-sweep, signal-conflicts, indicator-sweep, look-ahead]
tags: [sweep, signals, columns]
sources:
  - crates/barq/src/sweep.rs:42-186
  - crates/barq/examples/factor_sweep.rs
  - crates/barq/tests/factor_sweep.rs
order: 1201
---

# SignalSweep：因子到信号列

`SignalSweep` 是"参数和标的共用一条列轴"的最小完整表达：一个 `(时间, 标的)` 的因子矩阵、一组参数 case 和一条规则，展开成 `case × 标的` 列的进出场信号和对应的三级列标签，再交给 `FromSignals`。执行设置（费用、止损、仓位）不在这里，仍只有 `FromSignals` 一个来源。

## 位置

- `crates/barq/src/sweep.rs:42-123`（`from_factor`；标签 107–113）
- `crates/barq/src/sweep.rs:141-159`（`expand`）
- `crates/barq/src/sweep.rs:170-186`（`args`）
- `crates/barq/examples/factor_sweep.rs`（示例）

## 算法

```text
from_factor((name, &factor), symbols, cases, delay, rule):
    for case_idx, params in cases: for symbol: out_col = case_idx × n_symbols + symbol
        for row in delay..n_rows: score = factor[row − delay, symbol]；有限时 (entry, exit) = rule(score, params)
    labels：factor = repeat(name, n_cols)；case = repeat(cases, n_symbols)；symbol = tile(symbols, n_cases)
args(&expanded_close) → FromSignals { close, entries, exits, ..default }   ← 费用、止损用结构体更新语法补
```

它**不清洗**信号，让模拟器对真实持仓解冲突（见 [信号冲突解析链与 signals_to_size](../algorithms/signal-conflicts.md)）。示例 `crates/barq/examples/factor_sweep.rs`。

列号规则与 Python 侧 [`@barq.strategy`](../algorithms/strategy-sweep.md) 的 `p × S + s` 相同；设计说明见 [ARCHITECTURE §6.1](../../../docs/ARCHITECTURE.zh-CN.md)。

## 相关

- [barq：门面 crate](../crates/barq.md)：`SignalSweep` 是门面 crate 自己唯一的模块，经 `prelude` 导出。
- [列标签：Columns、Level 与 group_indices](../concepts/column-labels.md)：`Level::repeat` / `Level::tile` 生成的 `factor`/`case`/`symbol` 三级标签。
- [@barq.strategy 的参数扫描](../algorithms/strategy-sweep.md)：Python 侧同一条列轴。
- [信号冲突解析链与 signals_to_size](../algorithms/signal-conflicts.md)：不清洗的信号在模拟器里对真实持仓解冲突。
- [指标参数网格与共享缓存](../algorithms/indicator-sweep.md)：指标扫描使用同样的 `case × n_symbols + symbol` 列布局。
- [延迟与前视偏差](../concepts/look-ahead.md)：`delay` 让第 `row` 行只读 `factor[row − delay]`。
