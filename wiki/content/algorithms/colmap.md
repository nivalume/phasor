---
title: ColMap 稳定计数排序与 MappedArray
kind: algorithm
summary: ColMap::build 用三趟稳定计数排序按列分组记录，连续时零拷贝；MappedArray 的归约逐位复刻参考实现的 NaN 与平局规则。
parent: barq-records
related: [record-merge, trade-views, column-axis, parity-testing, order-statistics]
tags: [records, sorting, reduction]
sources:
  - crates/barq-records/src/mapped.rs:72-110
  - crates/barq-records/src/mapped.rs:175-181
  - crates/barq-records/src/mapped.rs:240-269
  - crates/barq-records/tests/parity_mapped.rs
order: 902
---

# ColMap 稳定计数排序与 MappedArray

记录是一条一条追加的，分析却总是"每列一组"：每列的交易数、每列的平均收益、每列收益最高的几笔。`ColMap` 把记录下标按列分组（与 CSR 布局同构），`MappedArray` 在一个按记录对齐的值数组上借助它做逐列归约。

## 位置

- `crates/barq-records/src/mapped.rs:72-110`（`ColMap::build`）
- `crates/barq-records/src/mapped.rs:175-181`（`ranges`）
- `crates/barq-records/src/mapped.rs:240-269`（`MappedArray::for_each_column`、`reduce`）

## ColMap::build：三趟稳定计数排序

```text
第一趟：校验 0 ≤ c < n_cols；lens[c] += 1；遇到 c < 上一个 c 就标记 contiguous = false
第二趟：starts = lens 的前缀和（独占）
第三趟：按原始顺序把记录下标散列到 idxs[starts[c] + filled[c]]   ← 按原顺序访问，所以稳定
```

## 快路径与慢路径

O(n + n_cols)。`ranges()` 只在 `contiguous` 时返回 `Some`，此时 `for_each_column` 直接把 `&values[start..end]` 交给闭包，零拷贝；否则收集到一个按 `max_col_len` 预分配、跨列复用的暂存区。两条路径结果相同（测试 `the_two_paths_agree`）。这个引擎按构造就是列主序发出，所以几乎总走快路径；慢路径给合并过、过滤过、外来的记录用。

## 归约的 NaN 与平局规则

`MappedArray` 的归约细节：`count` 连 NaN 一起数（唯一一个）；`sum` 全 NaN 给 0（同 `nansum`）；`std(ddof)` 先算总体方差再按 `n/(n−ddof)` 缩放，为了和参考实现逐位一致；`idxmax` 把 NaN 替换成 −inf 而不是跳过；`top_n_mask` 用 `select_nth_unstable_by` 找第 k 个顺序统计量（期望 O(n)），平局从**末尾**取，`bottom_n_mask` 从**开头**取——这是 vectorbt `top_n_inout_map_nb` 的不对称性，由 fixture 钉死。

这些规则由 28,367 行的 mapped fixture 钉住，见[差分测试与 fixture](../tooling/parity-testing.md)。

## 相关

- [barq-records：记录与派生视图](../crates/barq-records.md)：所属 crate；`ColMap` 与 `MappedArray` 在类图里的位置。
- [合并每线程缓冲与 order_id 重基](../algorithms/record-merge.md)：合并产生的记录是慢路径的来源之一。
- [三种交易视图](../algorithms/trade-views.md)：`column_ranges` 要求 `ColMap` 连续，否则 `Err`。
- [列轴与列主序](../concepts/column-axis.md)：为什么引擎按构造列主序发出、快路径几乎总能走。
- [差分测试与 fixture](../tooling/parity-testing.md)：平局规则与 NaN 规则的对照来源。
