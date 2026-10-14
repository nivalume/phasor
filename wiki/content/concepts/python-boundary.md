---
title: Python 边界：F 序与一维的三种含义
kind: concept
summary: columns_of 只接受列主序 NumPy 数组并零拷贝借用，C 序报错而非转置；一维数组的含义由调用点声明。
parent: phasor-py
related: [column-axis, flex-array, portfolio-cache, strategy-sweep, cash-flows]
tags: [python, numpy, layout]
sources:
  - crates/phasor-py/src/array.rs:1-145
  - crates/phasor-py/python/phasor/__init__.py:64-274
order: 1301
---

# Python 边界：F 序与一维的三种含义

引擎按[列主序](../concepts/column-axis.md)读数组，NumPy 默认行主序。`phasor-py` 的 Rust 侧借用 NumPy 的缓冲区而不复制，所以它必须确认缓冲区真的是列主序，并且要知道一个一维数组到底是"每行一个值"还是"每列一个值"，再把它翻译成 [`FlexArray`](../concepts/flex-array.md)。

## 位置

- `crates/phasor-py/src/array.rs:1-37`（模块文档：拒绝而不是转置）
- `crates/phasor-py/src/array.rs:82-108`（`columns_of`）
- `crates/phasor-py/src/array.rs:117-145`（`flex_f64`，配 `OneDim`）
- `crates/phasor-py/python/phasor/__init__.py:64-274`（`_matrix`、`_param`、`_flag`、`_accumulate`、`_group_lens`、`_column_map`）

## 数组零拷贝跨界与 F 序检查

```text
columns_of(array: &PyReadonlyArray2<f64>) -> (&[f64], n_rows, n_cols):
    f_ordered = n_rows == 1 || n_cols == 1 || strides[0] == 1       ← 列主序沿 axis 0 步长为 1（单位：元素）
    否则 ValueError，报出 shape、strides 和修法 np.asfortranarray(x)
    to_slice_memory_order() 失败（不连续）→ ValueError
```

为什么拒绝而不是转置：一个 `(3,2)` 的 `0..6` 数组 C 序是 `[0,1,2,3,4,5]`，F 序是 `[0,2,4,1,3,5]`；把 C 序字节按列主序读，第 0 列变成 `[0,1,2]` 而不是 `[0,2,4]`——**回测了转置后的序列却报出一个正常模样的结果**。检查放在 Rust 里，因为 `phasor._phasor` 可以被直接导入。

## 一维数组由调用点声明

一维数组的含义由调用点声明（`OneDim::{PerRow, PerColumn}`），"按长度猜多数时候对、少数时候无声地错"。整数数组被拒绝（当浮点看会拷贝）。`None` 表示"引擎默认"，由 `FromSignals::default()` 补，所以 Python 签名不重复任何默认值。

| 参数 | 标量 | 一维 | 二维 |
|---|---|---|---|
| 交易参数（size、fees、price、sl_stop…） | `Scalar` | `PerColumn`（长度 `n_cols`） | `Full`（`(n_rows, n_cols)`） |
| `cash_earnings`、非共享 `cash_deposits` | `Scalar` | `PerRow`（长度 `n_rows`） | `Full` |
| `init_cash` | `Scalar` | 按组（长度 `n_groups`） | 共享时拒绝 |
| 市场数据（close/open/high/low/cash_dividends） | — | 一列 | `(n_rows, source_cols)`，有 `column_map` 时 `FlexArray::mapped` |
| 共享 `cash_deposits` | `Scalar` | `PerRow` | `(n_rows, n_groups)`，`mapped(group_of)` |

Python 侧的 `_matrix`（1-D → **一列**，`asfortranarray`）、`_param`（Python 数字保持数字走免分配路径；1-D 只 `ascontiguousarray`，含义到 Rust 决定）、`_flag`、`_accumulate`（`True`→`both`）、`_group_lens`、`_column_map`。

`cash_earnings`、`cash_deposits` 与 `cash_dividends` 在模拟里怎么入账见 [成交之外的现金流](../concepts/cash-flows.md)；`column_map` 的映射语义见 [FlexArray 广播与列映射](../concepts/flex-array.md)。

## 相关

- [phasor-py：Python 层](../crates/phasor-py.md)：这里的检查与翻译是 Rust 侧三件事里的前两件。
- [列轴与列主序](../concepts/column-axis.md)：为什么引擎只读列主序缓冲区。
- [FlexArray 广播与列映射](../concepts/flex-array.md)：`Scalar`、`PerColumn`、`PerRow`、`Full`、`mapped` 这些形状的定义。
- [PyPortfolio 的跨调用缓存与 GIL](../concepts/portfolio-cache.md)：借用结束后，结果对象为什么要自己拥有一份收盘价。
- [@phasor.strategy 的参数扫描](../algorithms/strategy-sweep.md)：用 `column_map` 让源宽度的价格服务全部参数列。
