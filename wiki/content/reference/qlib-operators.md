---
title: 因子算子与 Qlib 对照
kind: reference
summary: 因子表达式语言的字段、运算符与函数一览，EMA 的参数约定，以及每个算子与 Qlib ops.py 的对应和对照精度。
related: [phasor-expr, pratt-parser, rolling-accumulators, alpha-feature-sets, polars-backend, look-ahead]
tags: [factors, qlib, operators]
sources:
  - crates/phasor-expr/src/graph.rs:423-437
  - crates/phasor-expr/src/eval.rs:686-730
  - crates/phasor-expr/tests/parity_qlib.rs
order: 30
---

# 因子算子与 Qlib 对照

[phasor-expr](../crates/phasor-expr.md) 接受的表达式语言是 Qlib 的方言：大多数公式可以原样照搬，少数地方（优先级、`Ref(x,0)`、WMA 权重）有意或无意地偏离。本页先列出语言本身，再逐算子对照 Qlib。

## 语言速览

本节摘自 [ARCHITECTURE §5.1](../../../docs/ARCHITECTURE.zh-CN.md#51-语言)；优先级与结合性的实现见 [Pratt 解析器](../algorithms/pratt-parser.md)。

- **字段** `$name`：OHLCV 来自 `from_ohlcv`，其他字段用 `with_field(name, &[f64])` 加入。
- **运算符**（优先级由低到高）：`|`、`&`、比较、`+ -`、`* /`、一元、`**`（右结合）。布尔结果为 `1.0`/`0.0`。

| 类别 | 函数 | 说明 |
|---|---|---|
| 逐元素 | `Abs Sign Log Not`；`Add Sub Mul Div Power Gt Ge Lt Le Eq Ne And Or`；`Greater Less`；`If(c, a, b)` | 运算符写法和函数写法是同一个节点 |
| 平移 | `Ref(x, n)`、`Delta(x, n)` | 按行；`Ref` 为负读取未来 → `uses_future()` |
| 平滑 | `EMA(x, p)` | `p < 1` 为 alpha，`p >= 1` 为 span，`0` 为扩展窗口 |
| 滚动 `(x, N)` | `Mean Sum Std Var Max Min Med Mad Count Skew Kurt WMA Rank IdxMax IdxMin Slope Rsquare Resi` | `Rank` 是时序排名；`IdxMax/Min` 从 1 开始 |
| 滚动 `(x, N, q)` | `Quantile` | 线性插值 |
| 成对 `(x, y, N)` | `Corr Cov` | |

窗口规则：`min_periods = 1`，`N = 0` 为扩展窗口，NaN 保持 NaN；参数必须是字面量。`alpha158()` / `alpha360()` 编译 Qlib 的特征集（需要显式的 `vwap`；不含归一化和标签）。暂不支持：截面算子、跨标的引用、掩码（[ARCHITECTURE §5.7](../../../docs/ARCHITECTURE.zh-CN.md#57-内存限制与计划)）。

## EMA

- **EMA**（`graph.rs:423-437`，`eval.rs:686-730`）：`p < 1` 当 alpha，`p ≥ 1` 当 span（`alpha = 2/(p+1)`），`p == 0` 扩展。实现是 pandas `ewm(adjust=True, ignore_na=False, min_periods=1)`：`num *= decay; den *= decay; 非 NaN 则 num += x; den += 1`。`EMA(x,0)` 是 Qlib 的自定义扩展权重，每行重算，**O(n²)**。

## 与 Qlib 的对应

| phasor | Qlib `qlib/data/ops.py` | 备注 |
|---|---|---|
| `$close` | `Feature("close")` | 小写化 |
| 算术/比较/逻辑算子与同名函数 | `Add Sub … Gt … And Or`（`PairOperator`） | 同一节点 |
| `Not Abs Sign Log`、`Greater Less`、`If` | `NpElemOperator`、`np.maximum/minimum`、`np.where` | |
| `Ref` / `Delta` | `series.shift(N)` / `x − Ref(x,N)` | N=0 不同 |
| `Mean Sum Std Var Max Min Count Med Skew Kurt Quantile` | pandas `rolling(N, min_periods=1).<fn>()` | 同 |
| `Mad` / `Rank` | `np.mean(np.abs(x − x.mean()))` / `percentileofscore(x, x[-1])/100` | 同 |
| `IdxMax IdxMin` | `rolling.apply(lambda x: x.argmax()+1)` | NaN 规则是 NumPy 的 |
| `Slope Rsquare Resi` | Cython `rolling.pyx` 的增量原始和 + `Rsquare` 的 2e-5 掩码 | 对照精度 1e-6 |
| `Corr Cov` | `rolling.corr/cov` + 任一边 std `isclose(0, atol=2e-5)` 掩码 | 对照精度 1e-4（Qlib 自己的增量和也有误差） |
| `EMA` | `ewm(alpha / span)`，`N=0` 自定义扩展权重 | 同 |
| `alpha158()` / `alpha360()` | `Alpha158DL` / `Alpha360DL.get_feature_config()` | 无标签、无处理器 |
| — | `Mask`、`ChangeInstrument`、`PRef`、`TResample`、`CSRankNorm` | 不支持 |

Qlib 按表达式字符串缓存、逐标的在 pandas Series 上求值；phasor 的对应物是哈希合并的图加按标的并行。Qlib 的 `get_extended_window_size`（"我需要多少历史"）没有对应：phasor 从你给的面板第一行起用 `min_periods=1`，预热行要调用方自己补。

各算子的实现在 [O(1) 滚动累加器](../algorithms/rolling-accumulators.md)、[单调队列](../algorithms/monotonic-queue.md)和[顺序统计量](../algorithms/order-statistics.md)；"哈希合并的图"与"按标的并行"分别见[依赖图编译](../algorithms/factor-graph.md)与[原生求值](../algorithms/native-evaluation.md)。

## 相关

- [phasor-expr：因子表达式引擎](../crates/phasor-expr.md)：这门语言的实现所在；它的阅读陷阱列出了与 Python/Qlib 的每处偏离。
- [Pratt 解析器](../algorithms/pratt-parser.md)：运算符优先级表；比较比 `&`/`|` 绑得紧，与 Python 相反。
- [O(1) 滚动累加器](../algorithms/rolling-accumulators.md)：`Slope Rsquare Resi Corr Cov` 的 2e-5 掩码与常量窗口处理。
- [Alpha158 与 Alpha360](../concepts/alpha-feature-sets.md)：`alpha158()` / `alpha360()` 怎样生成。
- [Polars 后端](../concepts/polars-backend.md)：Qlib fixture 同样在它上面跑。
- [延迟与前视偏差](../concepts/look-ahead.md)：负的 `Ref` 读未来，`uses_future()` 把它标出来。
