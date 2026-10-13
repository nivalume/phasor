---
title: PyPortfolio 的跨调用缓存与 GIL
kind: concept
summary: PyPortfolio 拥有记录与收盘价拷贝，每次调用用 from_borrowed 借出同一个 Cache 重建视图；引擎计算时释放 GIL。
parent: barq-py
related: [rebuild-chain, barq-analysis, python-boundary, sweep-aggregation, stats-registry]
tags: [python, cache, gil]
sources:
  - crates/barq-py/src/lib.rs:75-197
  - crates/barq-analysis/src/portfolio.rs:240-260
order: 1302
---

# PyPortfolio 的跨调用缓存与 GIL

引擎的 `Portfolio` 借用价格与记录，这在 Rust 里是对的；跨过 FFI 边界就不成立了，因为 Python 随时可能释放它的数组。`PyPortfolio` 因此自己拥有一份记录和收盘价，每次方法调用临时重建一个借用它们的引擎视图，并把派生结果存进一个跨调用保留的 `Cache`。

## 位置

- `crates/barq-py/src/lib.rs:75-197`（`PyPortfolio`、`core()` 127–151、`assemble()` 159–196）
- `crates/barq-analysis/src/portfolio.rs:240-260`（`Portfolio::from_borrowed`）

## 拥有什么，每次调用重建什么

Python 可能在调用返回后立即释放数组，所以 `PyPortfolio` **拥有**记录、源宽度的收盘价拷贝、初始现金、分组；每个方法：

```text
core():
    close = market_array(&self.close, n_rows, source_cols, column_map)      → FlexArray::full 或 ::mapped
    cash  = FlexArray::per_column(&self.init_cash, n_cols)
    CorePortfolio::from_borrowed(sorted_orders 或 orders, close, cash, n_rows, n_cols, Scoring{..}, groups, &self.cache)
```

`from_borrowed` 不克隆记录，派生结果落进出借的 `Cache`（13 个 `OnceLock`），所以一次调用算出的净值曲线供下一次用；`clear_cache()` 释放这些 `bars × accounts` 的矩阵。共享账户的记录在 `assemble` 时 `sort_by_column()` **一次**，存成 `sorted_orders`（整份克隆）。

## from_borrowed：分析层一侧

`Cache` 的 `OnceLock` 存的是[重建链](../algorithms/rebuild-chain.md)上的派生结果（持仓、现金、净值、收益、成交、回撤…）。分析层这一侧的入口在 `crates/barq-analysis/src/portfolio.rs`：

`from_borrowed(&OrderRecords, &Cache)`（240–260）是 Python 层的路径：Python 对象拥有价格与记录，每次调用重建一个借用它们的 `Portfolio<'a>`，派生结果留在调用方的 `Cache` 里供下次复用。它要求记录已经是列主序（不排序，因为排序要每次重复）。`with_scoring` 改 `trades_type` 时，借来的缓存会换成私有缓存而不是改动出借方的。

## GIL

GIL：`from_signals`/`from_orders`、`stats`、`metrics`、`sharpe_ratio`、`value`、`returns`、`cash`、`trades`、`_FactorSet.evaluate`、`crossed_above`、`_Dataset.prepare/align`、`load_data` 全部 `py.detach`；只有 NumPy 转换持有 GIL。`trades()` 在持 GIL 时先建 `core()` 再 detach。

## stats() 与按组取值

`stats()` 返回 dict：`Value::Count → int`、`Float → float`、`Missing → None`（NaN 是"算了但没答案"，None 是"输入没给"）；非共享时附带买入持有基准。`value/returns/cash(col=None, *, group=None)`：共享时必须用 `group=`，拒绝 `col=`。

统计表的行与 `Value` 的三种取值见 [统计表注册表](../concepts/stats-registry.md)。

## 相关

- [barq-py：Python 层](../crates/barq-py.md)：`PyPortfolio` 是 Rust 侧三件事里"释放 GIL 调引擎"的落点。
- [从记录重建净值](../algorithms/rebuild-chain.md)：`Cache` 里缓存的就是这条链上的序列。
- [barq-analysis：分析层](../crates/barq-analysis.md)：`Portfolio`、`Cache` 与 `from_borrowed` 的定义所在。
- [统计表注册表](../concepts/stats-registry.md)：`stats()` 返回的表从哪里来。
- [扫描结果汇总与 +inf 陷阱](../concepts/sweep-aggregation.md)：`traded_only` 为什么存在。
- [Python 边界：F 序与一维的三种含义](../concepts/python-boundary.md)：`close` 和 `init_cash` 在进入 `PyPortfolio` 之前怎么被借用和展开。
