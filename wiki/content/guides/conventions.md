---
title: 贯穿全仓库的约定
kind: guide
summary: 列轴、列主序、FlexArray 广播、浮点容差、拒单不是错误、记录是唯一来源、线程数不影响结果，外加"延迟"这条红线。
related: [column-axis, flex-array, float-tolerance, errors-vs-rejections, rebuild-chain, look-ahead]
tags: [conventions, overview]
sources:
  - crates/barq-core/src/columns.rs
  - crates/barq-core/src/matrix.rs
  - crates/barq-core/src/flex.rs
  - crates/barq-core/src/math.rs
  - crates/barq-exec/src/kernel.rs
  - crates/barq-analysis/src/series.rs
  - crates/barq-sim/src/random.rs
  - crates/barq-sim/src/from_orders.rs
  - crates/barq-signals/src/labels.rs
order: 20
---

# 贯穿全仓库的约定

这些约定在每个 crate 里都会反复出现，先记住它们，后面每一页都会少走很多弯路。

## 7 个约定

| # | 约定 | 一句话 | 第一次出现 | 详见 |
|---|---|---|---|---|
| 1 | **列轴** | 一列 = 一个被模拟的序列（一个标的，或一个 `(参数组, 标的)` 对）；`col = param_idx × n_symbols + symbol_idx`，参数**重复**（repeat）、标的**平铺**（tile） | `barq-core/src/columns.rs` | [列轴与列主序](../concepts/column-axis.md)、[列标签](../concepts/column-labels.md) |
| 2 | **列主序** | `(row, col)` 存在 `data[col * n_rows + row]`；一列是连续切片，所以"一列"是并行与缓存的工作单位；NumPy 数组进 Rust 必须是 Fortran 序 | `barq-core/src/matrix.rs` | [列轴与列主序](../concepts/column-axis.md)、[Python 边界](../concepts/python-boundary.md) |
| 3 | **`FlexArray` 广播** | 参数不物化：标量 / 按列 / 按行 / 列映射 / 全矩阵，五种变体同一个 `get(row, col)` | `barq-core/src/flex.rs` | [FlexArray 广播与列映射](../concepts/flex-array.md) |
| 4 | **浮点容差层** | 金融比较不用 `==`、`<=`；用 `is_close`/`is_close_or_less`/`is_less`，余额更新用 `add` 把接近 0 的结果钉成 0；`inf` 是活的哨兵（`size=+inf` = 全仓，`price=+inf` = 收盘价），所以 `inf` 永远不"接近" | `barq-core/src/math.rs` | [浮点容差与零钉](../concepts/float-tolerance.md) |
| 5 | **拒单不是错误** | `Err(Error::Invariant)` 表示不变量被破坏（负现金、坏参数），整个运行停止；撮合不成功只是 `OrderResult { status: Rejected/Ignored, status_info }` | `barq-exec/src/kernel.rs` | [错误与拒单](../concepts/errors-vs-rejections.md)、[拒单原因 StatusInfo](../reference/status-info.md) |
| 6 | **记录是唯一来源** | 分析层只从 `OrderRecords`（含 `CashRecords`）重建现金、持仓、净值；凡是改变现金或持仓的事情都必须落成记录 | `barq-records`、`barq-analysis/src/series.rs` | [从记录重建净值](../algorithms/rebuild-chain.md) |
| 7 | **线程数不影响结果** | 串行与多线程输出逐位一致；随机数由 `(seed, row, col)` 决定，不依赖线程或分组；每个并行点都有测试钉住这一点 | `barq-sim/src/random.rs`、`from_orders.rs` | [按账户组并行与确定性合并](../algorithms/parallel-simulation.md)、[无状态随机数 SplitMix64](../concepts/stateless-rng.md) |

## 延迟：研究层与信号层的红线

还有一条贯穿研究层与信号层的红线：**延迟**。在 `t` 的收盘价算出的因子，`t` 才知道，在 `t` 交易就是用了未来数据。所有信号工具默认延迟一根 K 线或把延迟当参数；`labels.rs` 里读未来的函数只能做训练目标，绝不能做信号。

详见 [延迟与前视偏差](../concepts/look-ahead.md)；`labels.rs` 里的函数在 [机器学习标签](../algorithms/ml-labels.md)。

## 相关

- [系统全景](../guides/system-map.md)：这些约定落在哪些 crate 上。
- [barq-core：基础类型](../crates/barq-core.md)：约定 1–4 都定义在这个 crate 里。
- [barq-exec：撮合内核](../crates/barq-exec.md)：约定 4 和 5 在内核里最要紧。
- [barq-sim：模拟循环](../crates/barq-sim.md)：约定 7 的并行点都在这里。
- [阅读路线](../guides/reading-roadmap.md)：记住约定之后从哪里开始读。
