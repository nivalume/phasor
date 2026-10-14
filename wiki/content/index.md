---
title: phasor 知识库
kind: guide
summary: phasor 源码知识库的首页：页面怎么组织、按角色从哪里读起、全部 crate、指南、参考与工具链页面的入口。
related: [system-map, conventions, reading-roadmap, algorithm-index, glossary]
tags: [index, reading]
order: 0
---

# phasor 知识库

> 这是一份"怎么读"的文档，不是"是什么"的文档。
> "是什么"在 [`docs/ARCHITECTURE.zh-CN.md`](../../docs/ARCHITECTURE.zh-CN.md)（设计与决策，自上而下）；
> 本知识库回答三个问题：**按什么顺序读、每段代码的核心算法是什么、看不懂时去哪里补课**。
> 代码以分支 `claude/kind-mayer-lp439i`（提交 `cffa4f1`）为准，所有 `path:line` 都指向这个版本。

phasor 是一个用 Rust 写的并行、因子驱动的向量化回测引擎，带一个 Python 包。项目介绍在 [README.md](../../README.md)。

## 知识库怎么组织

每页只讲一件事。页面分六类，每类一个目录：

| 类 | 目录 | 讲什么 |
|---|---|---|
| 指南 guide | `guides/` | 怎么读：全景、约定、路线、算法一览、练习、路线图 |
| crate | `crates/` | 一个 crate 一页：定位、文件与阅读顺序、类图、调用链、阅读陷阱、与参考实现的对应 |
| 概念 concept | `concepts/` | 跨函数的约定或数据结构，比如列轴、浮点容差、空头保证金 |
| 算法 algorithm | `algorithms/` | 一个核心算法一页：位置、思路、步骤、复杂度、数值技巧、为什么这么做 |
| 参考 reference | `reference/` | 查表用：拒单原因、指标公式、Qlib 对照、术语、延伸资料 |
| 工具链 tooling | `tooling/` | 引擎之外的设施：研究协议、差分测试、基准 |

crate 页的结构固定：**定位 → 文件与阅读顺序 → 类图 → 核心算法与概念 → 调用链 → 阅读陷阱 → 与参考实现的对应 → 延伸资料 → 自测**。
概念页和算法页先给位置（`path:line`），再讲内容，最后是"相关"。

页面之间有四种关系：

- **正文链接**：正文提到另一页的主题，就链过去。
- **`parent`（所属）**：每个概念页和算法页属于一个 crate 页或工具链页。crate 页的"核心算法与概念"一节列出它的全部子页。
- **`depends`（依赖）**：crate 页之间的依赖，照抄各 crate 的 `Cargo.toml`。
- **`related`（相关）**：另见，不分方向。

`wiki/` 下的 React 查看器把这四种关系画成一张图：节点是页面，边是关系，点一个节点就打开那一页。
不用查看器也能读：每页都是普通 Markdown，链接都是相对路径，在 GitHub 上直接能点。

## 从哪里读起

- **第一次读**：先看 [系统全景](guides/system-map.md)，记住 [贯穿全仓库的约定](guides/conventions.md)，再照 [阅读路线](guides/reading-roadmap.md) 从 [phasor-core](crates/phasor-core.md) 往上读。
- **读模拟器**：[phasor-exec](crates/phasor-exec.md) → [phasor-sim](crates/phasor-sim.md) → [phasor-records](crates/phasor-records.md) → [phasor-analysis](crates/phasor-analysis.md)。
  核心是 [execute_order 的固定顺序](algorithms/execute-order-pipeline.md)、[数量换算链](algorithms/size-cascade.md)、[一根 K 线](algorithms/bar-loop.md)、[止损引擎](algorithms/stop-engine.md) 和 [从记录重建净值](algorithms/rebuild-chain.md)。
- **读因子引擎**：[phasor-expr](crates/phasor-expr.md)。顺着 [Pratt 解析器](algorithms/pratt-parser.md) → [依赖图编译](algorithms/factor-graph.md) → [原生求值](algorithms/native-evaluation.md) → [滚动累加器](algorithms/rolling-accumulators.md) 读。
  因子变成信号在 [phasor-signals](crates/phasor-signals.md) 和 [SignalSweep](algorithms/signal-sweep.md)。
- **写 Python**：[phasor-py](crates/phasor-py.md)。先读 [Python 边界](concepts/python-boundary.md)：数组为什么必须是 F 序。
  再读 [跨调用缓存与 GIL](concepts/portfolio-cache.md)、[@phasor.strategy 的参数扫描](algorithms/strategy-sweep.md) 和 [时间序列交叉验证](algorithms/ts-cross-validation.md)。数据从 [phasor-data](crates/phasor-data.md) 进来。
- **查一个算法或一个词**：[核心算法一览](guides/algorithm-index.md)、[术语表](reference/glossary.md)。

## 全部页面

### 指南

- [系统全景](guides/system-map.md)：数据流、crate 依赖与分层、代码规模、全局类图。
- [贯穿全仓库的约定](guides/conventions.md)：7 个约定和"延迟"这条红线。
- [阅读路线](guides/reading-roadmap.md)：总路线、两条可选路线、用时规划、读一个 crate 的通用方法、阅读进度自测。
- [核心算法一览](guides/algorithm-index.md)：读完应该能脱稿讲清楚的算法，每项链到所在页。
- [练习与自测](guides/exercises.md)：每个 crate 的动手练习与自测题。
- [路线图与已知缺口](guides/roadmap.md)：`docs/plan.md` 的进度与已知问题。

### Crate（按依赖顺序，被依赖的在前）

- [phasor-core：基础类型](crates/phasor-core.md)：数组、广播、列标签、浮点容差、错误。
- [phasor-model：领域模型](crates/phasor-model.md)：领域枚举与 Id，和 vectorbt / Nautilus 对齐。
- [phasor-generic：通用窗口算子](crates/phasor-generic.md)：滚动 / 扩展 / EWM 窗口、合成价格。
- [phasor-signals：信号工具箱](crates/phasor-signals.md)：清洗、生成、排名、随机、标签。
- [phasor-indicators：技术指标](crates/phasor-indicators.md)：技术指标与参数网格。
- [phasor-data：数据层](crates/phasor-data.md)：Arrow 读取、面板、按 `available_time` 对齐。
- [phasor-expr：因子表达式引擎](crates/phasor-expr.md)：因子 DSL 全链路。
- [phasor-exec：撮合内核](crates/phasor-exec.md)：一笔订单对一个账户在一根 K 线上的撮合。
- [phasor-records：记录与派生视图](crates/phasor-records.md)：记录格式与派生视图。
- [phasor-sim：模拟循环](crates/phasor-sim.md)：驱动内核的两条循环、止损、账户组、并行。
- [phasor-analysis：分析层](crates/phasor-analysis.md)：从记录重建净值、全部指标。
- [phasor：门面 crate](crates/phasor.md)：prelude 与 `SignalSweep`。
- [phasor-py：Python 层](crates/phasor-py.md)：PyO3 扩展与 Python 包。

### 参考

- [拒单原因 StatusInfo](reference/status-info.md)：14 种拒单原因各在哪里触发。
- [指标公式表](reference/metrics.md)：分析层每个指标的公式。
- [因子算子与 Qlib 对照](reference/qlib-operators.md)：表达式语言的算子，和 Qlib 逐个对照。
- [术语表](reference/glossary.md)：术语、含义、详见哪一页。
- [延伸资料](reference/further-reading.md)：论文、文档、视频，中英文。

### 工具链

- [研究协议 phasor_lab](tooling/research-protocol.md)：预注册的因子研究协议，模型永远看不到结果。
- [差分测试与 fixture](tooling/parity-testing.md)：参考实现在外部生成 TSV fixture，`cargo test` 重放。
- [基准协议](tooling/benchmarks.md)：和 vectorbt 逐工作负载对比，同样的工作才比。

概念页和算法页不在这里逐个列出。每个 crate 页的"核心算法与概念"一节列出它的子页；[核心算法一览](guides/algorithm-index.md) 按算法列出全部。
