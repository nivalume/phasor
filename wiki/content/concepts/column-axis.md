---
title: 列轴与列主序
kind: concept
summary: (row, col) 存在 data[col * n_rows + row]；列号 = param_idx × n_symbols + symbol_idx，参数重复、标的平铺。
parent: barq-core
related: [column-labels, flex-array, indicator-sweep, parallel-simulation, python-boundary]
tags: [layout, columns, memory]
sources:
  - crates/barq-core/src/matrix.rs:1-12
  - crates/barq-core/src/matrix.rs:113
  - crates/barq-core/src/columns.rs:10-23
  - crates/barq-indicators/src/sweep.rs:135-137
order: 102
---

# 列轴与列主序

两条约定决定了 barq 里每个二维数组怎么存、每一列代表什么：存储按列主序，列轴按“参数组 × 标的”排列。[barq-core](../crates/barq-core.md) 的 `Matrix` 和 `Columns` 把它们固定下来，模拟、记录、因子和 Python 边界都照着用。

## 位置

- `crates/barq-core/src/matrix.rs:113`：索引公式
- `crates/barq-core/src/matrix.rs:1-12`：为什么列主序
- `crates/barq-core/src/columns.rs:10-23`：列轴布局
- `crates/barq-indicators/src/sweep.rs:135-137`：同一公式的 `column_of`

## 列主序

**索引公式**：`(row, col) ↦ data[col * n_rows + row]`，出现在 `matrix.rs:113`、`flex.rs:71`、`barq-data/src/lib.rs:388、548`。一列是 `data[col*n_rows .. (col+1)*n_rows]`，`Matrix::column(c)` 零拷贝返回它。

**为什么列主序**（`matrix.rs:1-12`）：列是工作单位——模拟扫一列（或相邻的一组），因子按标的（列）求值，一个线程拥有一段列。列连续，就没有跨线程共享的缓存行。

## 列轴布局

**列轴布局**（`columns.rs:10-23`）：`col = param_idx × n_symbols + symbol_idx`。

![列轴](../../../docs/img/zh/columns.svg)

```text
Level::repeat("window", [10,20,50], times=2)  → [10,10,20,20,50,50]    参数"重复"
Level::tile  ("symbol", [BTC,ETH],  times=3)  → [BTC,ETH,BTC,ETH,BTC,ETH] 标的"平铺"
```

结果：一组参数占据**相邻**的列，所以共享账户就是一个切片（`group_lens=[2,2,2]`）；同一标的的列是跨步的 `[0,2,4]`。
这只是构造函数约定，没有运行时检查（`columns.rs:21-23`），由测试 `one_parameter_combination_occupies_contiguous_columns`（399 行）钉住。
同一公式还出现在 `barq-indicators/src/sweep.rs:135-137` 的 `column_of`，以及 Python 里的 `column_map = np.tile(np.arange(S), n_params)`。

## 相关

- [barq-core：基础类型](../crates/barq-core.md)：`Matrix` 与 `Columns` 所在的 crate。
- [列标签：Columns、Level 与 group_indices](../concepts/column-labels.md)：`Level::repeat` / `tile` 给这套布局贴上标签。
- [FlexArray 广播与列映射](../concepts/flex-array.md)：同一索引公式；`MappedColumns` 把扫描列映射回物理列。
- [指标参数网格与共享缓存](../algorithms/indicator-sweep.md)：`column_of` 用的是同一公式。
- [按账户组并行与确定性合并](../algorithms/parallel-simulation.md)：相邻的列组成账户组，一个线程拥有一段列。
- [Python 边界：F 序与一维的三种含义](../concepts/python-boundary.md)：NumPy 数组进 Rust 必须是 Fortran 序。
