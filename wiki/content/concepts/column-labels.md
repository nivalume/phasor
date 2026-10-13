---
title: 列标签：Columns、Level 与 group_indices
kind: concept
summary: Columns 给每列贴分层标签而不复制数据；浮点标签按位相等，group_indices 按首次出现顺序把列号分组。
parent: barq-core
related: [column-axis, sweep-aggregation, errors-vs-rejections, flex-array]
tags: [labels, columns, grouping]
sources:
  - crates/barq-core/src/columns.rs:54-88
  - crates/barq-core/src/columns.rs:208-343
order: 104
---

# 列标签：Columns、Level 与 group_indices

一次参数扫描的输出是成千上万列数值，只看列号不知道哪一列是哪组参数、哪个标的。[barq-core](../crates/barq-core.md) 的 `Columns` 把这层含义单独存起来，供描述、分组和按参数聚合使用。

## 位置

- `crates/barq-core/src/columns.rs:54-88`：`Label` 的按位相等
- `crates/barq-core/src/columns.rs:151-179`：`Level::repeat` / `tile`
- `crates/barq-core/src/columns.rs:208-343`：`Columns`
- `crates/barq-core/src/columns.rs:305-325`：`group_indices`

## 列标签、`Columns` 与 `group_indices`（`columns.rs:54-88, 208-343`）

矩阵列保存数值；`Columns` 保存每一列“代表什么”的标签，不复制矩阵数据。标签按层级（`Level`）组织，例如参数扫描中的 `window` 和 `symbol`。每个层级必须恰好有 `n_cols` 个标签，否则 `Columns::new` 返回 `Error::LevelWidth`。

```rust
let windows = [10_i64, 20, 50];
let symbols = ["BTC", "ETH"];
let columns = Columns::new(vec![
    Level::repeat("window", &windows, symbols.len()),
    Level::tile("symbol", &symbols, windows.len()),
]).unwrap();
```

`repeat` 让每个参数值占相邻列，`tile` 让每个参数块都包含所有标的，因此列标签是：

| 列号 | window | symbol |
|---:|---:|---|
| 0 | 10 | BTC |
| 1 | 10 | ETH |
| 2 | 20 | BTC |
| 3 | 20 | ETH |
| 4 | 50 | BTC |
| 5 | 50 | ETH |

因此 `columns.describe(3)` 得到 `window=20, symbol=ETH`；`columns.group_indices("symbol")` 得到 BTC 对应 `[0, 2, 4]`、ETH 对应 `[1, 3, 5]`，可用于按标的聚合每列的收益等指标。查询不存在的层级会返回 `Error::UnknownLevel`。

`Label::key()` 把 `Float(v)` 折成 `(1, v.to_bits())`，`PartialEq` 和 `Hash` 都走 `key()`。于是 `Float(NaN) == Float(NaN)`、`Float(0.0) != Float(-0.0)`——和 IEEE 相反，但有意为之（文档 32–34 行："标签是标识符，不是测量值"），这样 `0.8, 0.9, 0.95` 这类分位数阈值才能当 `HashMap` 键。
`group_indices(level)`：一趟扫描建 `HashMap<Label, Vec<usize>>`，同时记录首次出现顺序，再按该顺序输出。它是[扫描结果汇总](../concepts/sweep-aggregation.md)按参数聚合（`aggregate.rs:144`）的基础。

## 相关

- [barq-core：基础类型](../crates/barq-core.md)：`Columns`、`Level`、`Label` 所在的 crate。
- [列轴与列主序](../concepts/column-axis.md)：`repeat` / `tile` 产生的列轴布局，以及它为什么让共享账户成为一个切片。
- [扫描结果汇总与 +inf 陷阱](../concepts/sweep-aggregation.md)：`group_indices` 是按参数聚合的基础。
- [错误与拒单](../concepts/errors-vs-rejections.md)：`LevelWidth`、`UnknownLevel` 是不变量错误。
- [FlexArray 广播与列映射](../concepts/flex-array.md)：同一套列轴上的参数怎么广播。
