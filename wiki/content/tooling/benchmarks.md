---
title: 基准协议
kind: tooling
summary: 与 vectorbt 的 Numba / Rust 后端逐工作负载对比：校验和保证同样的工作，只比内核层，best-of-7，基线门槛，比例报区间。
related: [parity-testing, bar-loop, parallel-simulation, roadmap, research-protocol]
tags: [benchmarks, performance]
sources:
  - benchmarks/README.md
  - benchmarks/compare.py
  - benchmarks/workloads.tsv
  - benchmarks/bench_vectorbt.py
  - crates/phasor-sim/examples/bench.rs
order: 30
---

# 基准协议

`benchmarks/` 回答两个问题：phasor 的模拟循环是不是真的比 vectorbt 快、快多少；一次改动有没有让自己变慢。协议的重点不在测得多快，而在"比的是同样的工作"和"报出来的比例是诚实的"。它和[研究协议 phasor_lab](../tooling/research-protocol.md)、[差分测试与 fixture](../tooling/parity-testing.md)一起，是"围绕引擎的三件外围设施"。

## 定位

- **`benchmarks`**：和 vectorbt 的 Numba / Rust 后端逐工作负载对比的协议，重点是"同样的工作"（校验和）和"诚实的比例"（核对核、best-of-7、基线门槛）。

## 文件与阅读顺序

| 顺序 | 文件 | 行数 | 读什么 |
|---|---|---:|---|
| 10 | `benchmarks/README.md`、`benchmarks/compare.py` | — | 协议与门槛 |

顺序号沿用研究协议、差分测试与基准的整体阅读顺序（其余各行见 [研究协议 phasor_lab](../tooling/research-protocol.md#文件与阅读顺序)）。其余文件：`crates/phasor-sim/examples/bench.rs`（phasor 的 runner）、`benchmarks/bench_vectorbt.py`（vectorbt 的 runner）、`benchmarks/bench_python.py`（Python 层的开销）、`benchmarks/workloads.tsv` 与 `benchmarks/workloads_signals.tsv`（工作负载）、`benchmarks/baseline.tsv`（基线门槛的参照），运行命令在 `benchmarks/README.md` 开头。

## 协议规则

| 规则 | 为什么 |
|---|---|
| 两个 runner 读同一份 `workloads.tsv`，用逐位相同的公式生成输入 | 可比 |
| 每个 runner 报 `checksum`（close 与 size 位模式的 wrapping u64 和）和成交数；`compare.py` 不一致就退出码 2 | "同样的工作，否则不比" |
| 只比 `kernel` 层（`simulate_from_orders_nb` 对 phasor 循环），`api` 层的包装约 2× | 诚实的比例 |
| Rust 侧 best-of-7 | 单线程噪声地板 ~1.3%（best-of-3 最差 9.5%） |
| `--check-baseline`：只看单线程行，容差 10%；`looks_uniform` 检测"整机一起慢" | 多线程离散度曾达 47% |
| 比例是区间不是数字 | Numba 循环在忙机器上掉得比 phasor 多，机器越空 phasor 越好看 |

## 结果

结果（4 核 Xeon，三次 sitting）：对 vectorbt Numba 2.6×–3.3×，对 vectorbt Rust 3.5×–4.9×，4 线程对 Numba 9.7×–12.1×，vectorbt Rust 对 Numba 0.74×–0.82×。`plan.md` P1 记录了 `from_signals` 在新功能全关时慢 3.8%（超过 3% 约束）和待做的 callgrind 指令数回归门槛（见 [路线图与已知缺口](../guides/roadmap.md)）。

被测的循环是 [from_orders 的一根 K 线](../algorithms/bar-loop.md)；"4 线程"一行的并行方式见 [按账户组并行与确定性合并](../algorithms/parallel-simulation.md)。

## 延伸资料

- 基准方法：Mytkowicz et al., "Producing Wrong Data Without Doing Anything Obviously Wrong!"（ASPLOS 2009）——为什么要 best-of-N 和"整机一起慢"的检测；callgrind 文档。
- 更多见 [延伸资料](../reference/further-reading.md) 的研究协议、验证、基准部分。

## 自测

1. `compare.py` 的校验和为什么要包含 `size` 而不只是 `close`？

## 相关

- [差分测试与 fixture](../tooling/parity-testing.md)：先证明结果一致，速度对比才有意义；两边共用 `.venv-parity`。
- [from_orders 的一根 K 线](../algorithms/bar-loop.md)：`kernel` 层计时的就是这个循环。
- [按账户组并行与确定性合并](../algorithms/parallel-simulation.md)：多线程行为什么能与单线程逐位一致。
- [路线图与已知缺口](../guides/roadmap.md)：P1 性能约束与 K4 基准覆盖。
