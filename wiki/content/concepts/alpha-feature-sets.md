---
title: Alpha158 与 Alpha360
kind: concept
summary: alpha158() 与 alpha360() 按 Qlib 的特征配置生成 158 和 360 个因子公式并编译成一个 FactorSet，不含标签与标准化，vwap 由调用方提供。
parent: barq-expr
related: [factor-graph, qlib-operators, order-statistics, monotonic-queue, polars-backend]
tags: [factors, qlib, features]
sources:
  - crates/barq-expr/src/alpha158.rs
  - crates/barq-expr/src/alpha360.rs
  - crates/barq-expr/tests/parity_qlib.rs
order: 707
---

# Alpha158 与 Alpha360

Qlib 的两套默认特征库在 barq 里是两个返回 `FactorSet` 的函数：公式按 Qlib `Alpha158DL` / `Alpha360DL` 的 `get_feature_config()` 生成字符串，再走普通的[编译](../algorithms/factor-graph.md)与[求值](../algorithms/native-evaluation.md)。它们是 barq-expr 的主要负载，也是 Qlib fixture 钉住的对象。

## 位置

- `crates/barq-expr/src/alpha158.rs`：`alpha158()`
- `crates/barq-expr/src/alpha360.rs`：`alpha360()`
- `crates/barq-expr/tests/parity_qlib.rs`：用 Qlib 预言机生成的 fixture（`tests/fixtures/qlib_alpha158.tsv`、`tests/fixtures/qlib_alpha360.tsv`）对照

## 生成方式

**Alpha158**（`alpha158.rs`）：9 个 K 线形态 + 4 个价格比 + 5 个窗口 × 29 族 = 158；先按窗口生成再按算子重排以匹配 Qlib 的 `ROC5..ROC60, MA5..` 顺序。**Alpha360**：5 个价格字段 × 60 个滞后的 `Ref($f,k)/$close` + 60 个 `Ref($volume,k)/($volume+1e-12)`。两者都不含标签与标准化处理器，`vwap` 必须由调用方提供。

## 编译后的规模

两套特征集编译后的节点、槽位、直写输出与视图个数（Alpha158：357 个节点、17 个槽位；Alpha360：722 个节点、1 个槽位、354 个视图）以及这些数字的来由，见依赖图编译页的[实测：节点与槽位](../algorithms/factor-graph.md#实测节点与槽位)。

## 相关

- [barq-expr：因子表达式引擎](../crates/barq-expr.md)：所属 crate；`alpha158.rs`、`alpha360.rs` 是它阅读顺序里的第 7 步。
- [依赖图编译：哈希合并、Ref 视图与槽位调度](../algorithms/factor-graph.md)：哈希合并让几十个因子共用一个 `Ref($close,1)`，Alpha360 的 354 个 `Ref` 都成了视图。
- [因子算子与 Qlib 对照](../reference/qlib-operators.md)：`alpha158()` / `alpha360()` 对应 `Alpha158DL` / `Alpha360DL.get_feature_config()`，以及逐算子的对照精度。
- [顺序统计量：有序环与树状数组](../algorithms/order-statistics.md)：Alpha158 的所有窗口 ≤ 60，所以 `QTLU`/`QTLD`/`RANK` 永远走有序环。
- [单调队列：窗口极值与极值位置](../algorithms/monotonic-queue.md)：`MAX`/`MIN`/`IMAX`/`IMIN` 族的核。
- [Polars 后端](../concepts/polars-backend.md)：Qlib fixture 在两个后端上都跑，表里也有两套特征集的耗时对比。
