---
title: Polars 后端
kind: concept
summary: 可选的 polars 特性提供第二个 ComputeBackend：直接把语法树编译成按标的分组的 Polars 惰性表达式，用来独立交叉验证原生后端，慢 3–15×。
parent: barq-expr
related: [native-evaluation, qlib-operators, alpha-feature-sets, parity-testing, benchmarks]
tags: [polars, backend, verification]
sources:
  - crates/barq-expr/src/polars_backend.rs
  - crates/barq-expr/src/backend.rs
  - crates/barq-expr/tests/parity_qlib.rs
order: 708
---

# Polars 后端

`ComputeBackend` 是一个只有 `name()` 和 `evaluate()` 的 trait；除了[原生求值](../algorithms/native-evaluation.md)的 `NativeBackend`，`polars` 特性还提供 `PolarsBackend`。它和原生后端共享解析结果但不共享依赖图，所以能作为一份独立实现去交叉验证原生后端，而不是更快的替代品。

## 位置

- `crates/barq-expr/src/polars_backend.rs`：`PolarsBackend`（`polars` 特性）
- `crates/barq-expr/src/backend.rs`：`ComputeBackend` trait 与 `NativeBackend`
- `crates/barq-expr/tests/parity_qlib.rs`：`polars_matches_qlib_alpha158_fixture` 等 Qlib fixture 测试的 Polars 版本

## 实现

**Polars 后端**（`polars_backend.rs`）：长表 + `__barq_expr_symbol` 列，每个因子表达式 `.over(symbol)`，64 个因子一批 `lazy().select().collect()`。直接编译语法树而不是图，所以没有槽位/视图。`Mean Std Sum Max Min Var Quantile Med EMA Cov` 用 Polars 原生滚动，`Rank IdxMax IdxMin Slope Rsquare Resi Skew Kurt Mad Count WMA` 用 `rolling_map` 调 `eval.rs` 里的参考归约器，`Corr` 用 `map_multiple` 每窗重算以得到 2e-5 掩码。用途：独立交叉验证（Qlib fixture 在两个后端上都跑）。慢 3–15×。

这些参考归约器（`eval.rs:590-849`）只在 `cfg(any(test, feature = "polars"))` 下编译，见 [barq-expr 的阅读陷阱](../crates/barq-expr.md#阅读陷阱)第 11 条；[路线图](../guides/roadmap.md)说这个后端"只在下游需要 Polars 时存在"。

## 启用与性能

用法与下面的性能对比来自 [ARCHITECTURE §5.6](../../../docs/ARCHITECTURE.zh-CN.md#56-polars-后端)。启用 `polars` feature 后：`set.evaluate_with(&inputs, &PolarsBackend)`。

| 面板 | 负载 | 原生 | Polars | 倍数 |
|---|---|---:|---:|---:|
| 1024 × 16 | 4 个逐元素因子 | 0.3 ms | 5.0 ms | 15× |
| 2520 × 200 | 6 个滚动因子（w=20） | 31 ms | 102 ms | 3.3× |
| 2520 × 200 | Alpha158 | 0.60 s | 6.2 s | 10× |
| 2520 × 200 | Alpha360 | 0.41 s | 2.7 s | 6.7× |
| 2520 × 500 | Alpha158 | 1.5 s | 19.4 s | 13× |

（4 线程，取 3 次最好。）开销来自：每个算子都要重新分组、O(window) 的 UDF、批次之间无法共享、`Option<f64>` 转换。

## 相关

- [barq-expr：因子表达式引擎](../crates/barq-expr.md)：所属 crate；类图里 `ComputeBackend` 的两个实现。
- [原生求值：按标的并行走图](../algorithms/native-evaluation.md)：被交叉验证的默认后端；它有图、槽位与视图，这里没有。
- [因子算子与 Qlib 对照](../reference/qlib-operators.md)：两个后端都要满足的 Qlib 语义。
- [Alpha158 与 Alpha360](../concepts/alpha-feature-sets.md)：性能表里的两个主要负载，也是 Qlib fixture 的内容。
- [差分测试与 fixture](../tooling/parity-testing.md)：Qlib fixture 怎样生成、怎样在两个后端上重放。
- [基准协议](../tooling/benchmarks.md)：性能数字应当怎样测量与报告。
