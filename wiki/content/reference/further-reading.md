---
title: 延伸资料
kind: reference
summary: 按领域列出的论文、书、文档、博客、视频和源码，每条注明类型、语言和为什么读。
related: [glossary, exercises, algorithm-index, research-protocol, parity-testing]
tags: [references, reading]
order: 50
---

# 延伸资料

按领域分节，每节开头链接对应的 crate 页面。每条给出：**类型**（论文 / 书 / 文档 / 博客 / 视频 / 源码）、**语言**、**为什么读**。
只列对理解本仓库有直接帮助的；泛泛的教材只在没有更精确来源时才出现。
链接在 2026-10 核对过可达性；SSRN 和期刊链接有时需要登录，可用标题检索替代。

## 基础层

对应页面：[barq-core](../crates/barq-core.md) · [barq-model](../crates/barq-model.md)

| 资料 | 类型 / 语言 | 为什么读 |
|---|---|---|
| PEP 485 – A Function for testing approximate equality：<https://peps.python.org/pep-0485/> | 文档 / 英 | [`is_close_tol`](../concepts/float-tolerance.md) 的四行就是它；理解为什么相对容差取两者绝对值的最大值、零附近为什么需要绝对容差 |
| David Goldberg, "What Every Computer Scientist Should Know About Floating-Point Arithmetic"（1991） | 论文 / 英 | 灾难性抵消一节解释 `is_addition_zero` 为什么比绝对值而不是比和 |
| Wikipedia "Row- and column-major order"；NumPy 文档 "Internal memory layout of an ndarray"（`order='F'`、`flags.f_contiguous`） | 文档 / 英 | [列主序](../concepts/column-axis.md)与 Fortran 序的全部背景 |
| Mike Acton, "Data-Oriented Design and C++"（CppCon 2014 主题演讲）：<https://isocpp.org/blog/2015/01/cppcon-2014-data-oriented-design-and-c-mike-acton>（YouTube 视频 ID `rX0ItVEVjHc`） | 视频 / 英 | `OhlcvPanel` 为什么是 SoA、记录为什么是 SoA |
| Richard Fabian, *Data-Oriented Design*（在线免费书） | 书 / 英 | 同上，更系统 |
| NumPy 文档 "Broadcasting"；vectorbt 文档 `base.reshape_fns`（`flex_select_nb`） | 文档 / 英 | [`FlexArray`](../concepts/flex-array.md) 的心智模型 |
| Rust 文档 `f64::total_cmp`、IEEE 754-2008 §5.10 totalOrder | 文档 / 英 | [`Label::Float`](../concepts/column-labels.md) 按位比较、`order.rs` 里 `-0.0` 与 `0.0` 的区分 |
| NautilusTrader 文档 "Instruments"、"Bars"、"Value types"：<https://nautilustrader.io/docs/latest/concepts/data/bar/>、<https://nautilustrader.io/docs/latest/concepts/instruments> | 文档 / 英 | `BarType` 字符串语法、`Price`/`Quantity` 定点数与精度模式的来源 |
| 《Rust 语言圣经》（course.rs）"枚举与模式匹配"、"生命周期"、"并发"章节：<https://course.rs> | 书 / 中 | 读 `FlexArray<'a, T>`、作用域线程前的 Rust 基础 |

## 撮合内核

对应页面：[barq-exec](../crates/barq-exec.md)

| 资料 | 类型 / 语言 | 为什么读 |
|---|---|---|
| vectorbt 文档 `portfolio.base`（`Portfolio.from_orders` 的参数表）与 `portfolio.nb`：<https://vectorbt.dev/api/portfolio/base/>、<https://vectorbt.dev/api/portfolio/nb/> | 文档 / 英 | [`size_type`](../algorithms/size-cascade.md)、`direction`、[`lock_cash`](../algorithms/buy-leg.md)、`allow_partial`、`size_granularity` 的语义；`execute_order_nb` 是[内核](../algorithms/execute-order-pipeline.md)的参考 |
| vectorbt 源码 `vectorbt/portfolio/nb.py`（1.1.0）、`vectorbt/portfolio/enums.py`、`vectorbt/utils/math_.py` | 源码 / 英 | 逐函数对照：`buy_nb`、[`sell_nb`](../algorithms/sell-leg.md)、[`OrderStatusInfo`](../reference/status-info.md)、`is_close_nb`、`add_nb` |
| Investopedia "Short Selling: Margin Requirements"、Reg T | 文档 / 英 | [`debt` / `free_cash` / `2 × short_value`](../concepts/short-margin.md) 的现实对应 |
| Pitest "Basic concepts"；`cargo-mutants` 文档 | 文档 / 英 | [变异测试](../tooling/parity-testing.md)与"幸存者"概念；[浮点容差与零钉](../concepts/float-tolerance.md)里的故事 |

## 模拟循环

对应页面：[barq-sim](../crates/barq-sim.md)

| 资料 | 类型 / 语言 | 为什么读 |
|---|---|---|
| vectorbt 文档 `Portfolio.from_signals` 参数说明（`accumulate`、[`upon_*_conflict`](../algorithms/signal-conflicts.md)、`upon_opposite_entry`、[`sl_stop`/`sl_trail`/`tp_stop`](../algorithms/stop-engine.md)、`stop_entry_price`/`stop_exit_price`/`upon_stop_exit`/`upon_stop_update`）；"Grouping / cash sharing / [call_seq](../algorithms/call-seq-auto.md)" 小节 | 文档 / 英 | [barq-sim](../crates/barq-sim.md) 每个枚举的用户视角解释 |
| Steele, Lea & Flood, "Fast Splittable Pseudorandom Number Generators"（OOPSLA 2014，DOI 10.1145/2660193.2660195）：<https://2014.splashcon.org/event/oopsla2014-fast-splittable-pseudorandom-number-generators> | 论文 / 英 | [SplitMix64](../concepts/stateless-rng.md) 的输出函数；为什么它是 64 位双射 |
| Salmon, Moraes, Dror & Shaw, "Parallel Random Numbers: As Easy as 1, 2, 3"（SC 2011） | 论文 / 英 | "随机数是坐标的函数"（计数器式 RNG）这一思想，`uniform(seed,row,col)` 的理论背景 |
| Rust 标准库文档 `std::thread::scope`（RFC 3151，Rust 1.63） | 文档 / 英 | [作用域线程](../algorithms/parallel-simulation.md)为什么强制 `Sync` 输入与列本地状态 |
| Investopedia "Stop-Loss vs. Stop-Limit"、"Gap Risk"；"Trailing Stop" | 文档 / 英 | `stop_price` 的跳空分支与 trailing 的 high-water mark |
| Robert Carver, *Systematic Trading*（2015）第 10–11 章 | 书 / 英 | 按目标仓位与波动率定仓位的实践背景（`TargetPercent`） |

## 记录

对应页面：[barq-records](../crates/barq-records.md)

| 资料 | 类型 / 语言 | 为什么读 |
|---|---|---|
| vectorbt 文档 "Trades"（EntryTrades / ExitTrades / Positions）、"Records"、"MappedArray"：<https://vectorbt.dev/api/portfolio/trades/>、<https://vectorbt.dev/api/records/> | 文档 / 英 | [三种视图](../algorithms/trade-views.md)、`col_map`、`top_n` 平局规则的来源 |
| Interactive Brokers "FIFO vs. LIFO vs. Average Cost" 之类的券商说明 | 文档 / 英 | 交易匹配口径 |
| John Sweeney, *Maximum Adverse Excursion*（1996）；Curtis Faith, *Way of the Turtle*（2007，中译《海龟交易法则》）E-ratio 一节 | 书 / 英、中 | [MAE/MFE 与 `edge_ratio`](../algorithms/excursions.md) |
| CLRS 第 8.2 节"计数排序"；OI Wiki "计数排序" | 书 / 英、中 | [`ColMap::build`](../algorithms/colmap.md) |
| Rust 标准库 `slice::split_at_mut`、`chunks_mut` | 文档 / 英 | 无 `unsafe` 的[并行拷贝](../algorithms/record-merge.md) |

## 分析

对应页面：[barq-analysis](../crates/barq-analysis.md)

| 资料 | 类型 / 语言 | 为什么读 |
|---|---|---|
| Bailey & López de Prado, "The Sharpe Ratio Efficient Frontier"（*Journal of Risk*, 2012） | 论文 / 英 | [PSR](../algorithms/overfitting-stats.md) 的推导（偏度、峰度修正） |
| Bailey & López de Prado, "The Deflated Sharpe Ratio: Correcting for Selection Bias, Backtest Overfitting and Non-Normality"（*JPM*, 2014）：<https://papers.ssrn.com/abstract=2460551> | 论文 / 英 | `expected_max_sharpe` 的极值公式与 DSR |
| Bailey, Borwein, López de Prado & Zhu, "The Probability of Backtest Overfitting"（*J. Computational Finance*, 2017）：<https://papers.ssrn.com/abstract=2326253>；开放版本 <https://escholarship.org/content/qt4w1110bb/qt4w1110bb.pdf> | 论文 / 英 | CSCV 与 PBO 的定义，`pbo_cscv` 逐步对照 |
| López de Prado, *Advances in Financial Machine Learning*（Wiley, 2018）第 11–14 章；SSRN 上的 Cornell ORIE 5256 课程幻灯片（"Lecture 1/10" … 共 9 讲）；同作者《面向资产管理者的机器学习》中译本（机械工业出版社，2021） | 书、幻灯片 / 英、中 | 回测统计的教科书形式 |
| Peter J. Acklam 的逆正态 CDF 算法（原页已失效，Wayback 有存档；二手讲解：<https://stackedboxes.org/2017/05/01/acklams-normal-quantile-function/>） | 博客 / 英 | `norm_ppf` 的系数、`P_LOW = 0.02425`、Halley 精化 |
| Abramowitz & Stegun §7.1；Numerical Recipes §6.2 | 书 / 英 | `erfc` 的级数与 Laplace 连分式 |
| Keating & Shadwick, "A Universal Performance Measure"（2002）；Sortino & van der Meer (1991)；Terry Young, "Calmar Ratio"（*Futures*, 1991）；Rockafellar & Uryasev (2000) | 论文 / 英 | [Omega、Sortino、Calmar、CVaR](../reference/metrics.md) 的原始定义 |
| Hyndman & Fan, "Sample Quantiles in Statistical Packages"（*American Statistician*, 1996） | 论文 / 英 | `percentile` 的 type 7 |
| Van Tharp, *Trade Your Way to Financial Freedom*（中译《通向财务自由之路》） | 书 / 英、中 | SQN、期望值 |
| MacKinlay, "Event Studies in Economics and Finance"（*JEL*, 1997） | 论文 / 英 | [`events.rs`](../algorithms/event-study.md) 的方法论 |
| Quantopian `empyrical` 源码 | 源码 / 英 | `vectorbt.returns.nb` 的公式血统 |

## 因子引擎

对应页面：[barq-expr](../crates/barq-expr.md)

| 资料 | 类型 / 语言 | 为什么读 |
|---|---|---|
| Alex Kladov（matklad），"Simple but Powerful Pratt Parsing"（2020）：<https://matklad.github.io/2020/04/13/simple-but-powerful-pratt-parsing.html> | 博客 / 英 | [`parse.rs`](../algorithms/pratt-parser.md) 的 `(left_bp, right_bp)` 写法直接来自这里；**读 `parse.rs` 前先读它** |
| Bob Nystrom, *Crafting Interpreters* 第 17 章 "Compiling Expressions"：<https://craftinginterpreters.com/compiling-expressions.html>（有社区中译） | 书 / 英、中 | Pratt 解析的另一种讲法，更循序渐进 |
| Vaughan Pratt, "Top Down Operator Precedence"（POPL 1973） | 论文 / 英 | 原始论文 |
| Filliâtre & Conchon, "Type-Safe Modular Hash-Consing"（ML Workshop 2006） | 论文 / 英 | [哈希合并](../algorithms/factor-graph.md) |
| 龙书 *Compilers: Principles, Techniques, and Tools* §6.1.2（表达式的 DAG）、§9.4（常量折叠）；中译《编译原理》 | 书 / 英、中 | `Builder::intern` / `fold` |
| Poletto & Sarkar, "Linear Scan Register Allocation"（TOPLAS 1999） | 论文 / 英 | `Graph::schedule` 的"最后读者释放槽位"就是线性扫描的活跃区间到期 |
| Wikipedia "Algorithms for calculating variance" → "Computing shifted data"；Chan, Golub & LeVeque (1983)；Welford (1962) | 文档、论文 / 英 | [`Moments.shift`](../algorithms/rolling-accumulators.md)、周期重建、与 Welford 的对比 |
| Higham, *Accuracy and Stability of Numerical Algorithms* 第 4 章 | 书 / 英 | 求和误差为什么线性增长、重建为什么能把它限住 |
| pandas 文档 "Windowing operations"、"Exponentially weighted windows"；pandas 源码 `_libs/window/aggregations.pyx` | 文档、源码 / 英 | `min_periods`、`adjust`、`ignore_na`；pandas 自己也是加减 + 周期校正 |
| Joanes & Gill, "Comparing measures of sample skewness and kurtosis"（1998） | 论文 / 英 | `Shape` 的 G1/G2 公式 |
| OI Wiki "单调队列"：<https://oi-wiki.org/ds/monotonous-queue/>；LeetCode 239 Sliding Window Maximum | 文档 / 中、英 | [`rolling_extremum_1d`、`rolling_arg_extreme_into`](../algorithms/monotonic-queue.md) |
| OI Wiki "树状数组"：<https://oi-wiki.org/ds/fenwick/>（含"第 k 小"的倍增）；cp-algorithms "Fenwick Tree"；Fenwick (1994) | 文档、论文 / 中、英 | [`Ordered::kth`](../algorithms/order-statistics.md) 的二进制提升 |
| Niko Matsakis, "Rayon: data parallelism in Rust"（2015）：<https://smallcultfollowing.com/babysteps/blog/2015/12/18/rayon-data-parallelism-in-rust/>；rayon 文档 `for_each_init` | 博客、文档 / 英 | [`evaluate`](../algorithms/native-evaluation.md) 的并行结构 |
| Qlib 文档 "Data Layer"（表达式与算子）：<https://qlib.readthedocs.io/en/latest/component/data.html>；源码 `qlib/data/ops.py`、`qlib/data/_libs/rolling.pyx`、`qlib/contrib/data/loader.py` | 文档、源码 / 英 | [每个算子的参考语义](../reference/qlib-operators.md)；[Alpha158/360](../concepts/alpha-feature-sets.md) 的配置 |
| Yang et al., "Qlib: An AI-oriented Quantitative Investment Platform"（arXiv 2020） | 论文 / 英 | Qlib 的整体设计 |
| Kakushadze, "101 Formulaic Alphas"（arXiv 1601.00991，*Wilmott* 2016）：<https://arxiv.org/abs/1601.00991> | 论文 / 英 | 因子表达式库这一体裁；[路线图](../guides/roadmap.md) H4 |

## 通用算子、指标、信号

对应页面：[barq-generic](../crates/barq-generic.md) · [barq-indicators](../crates/barq-indicators.md) · [barq-signals](../crates/barq-signals.md)

| 资料 | 类型 / 语言 | 为什么读 |
|---|---|---|
| J. Welles Wilder, *New Concepts in Technical Trading Systems*（1978） | 书 / 英 | RSI、TR/ATR 的原始定义，对照 barq 的 Cutler/EMA 变体 |
| John Bollinger, *Bollinger on Bollinger Bands*（2001）；Gerald Appel 的 MACD；Joseph Granville 的 OBV | 书 / 英 | 指标定义 |
| pandas `ewm` 文档；vectorbt `IndicatorFactory` 文档、`signals.accessors` 文档 | 文档 / 英 | [`adjust`/`ignore_na`](../algorithms/generic-windows.md)；[参数网格与列堆叠](../algorithms/indicator-sweep.md)；[`pos_rank`、`partition_ranges`](../algorithms/signal-rank-ranges.md) |
| Knuth, TAOCP 卷 2 算法 P（Fisher–Yates / Durstenfeld） | 书 / 英 | [`random_n`](../algorithms/random-signals.md) 的部分洗牌 |
| Glasserman, *Monte Carlo Methods in Financial Engineering*（2003）第 3 章 | 书 / 英 | [GBM 精确离散化](../algorithms/synthetic-prices.md) vs Euler–Maruyama；Box–Muller |
| López de Prado, *AFML* 第 3 章（三重障碍、元标签）、第 7 章（purge、embargo） | 书 / 英 | [`triple_barrier`](../algorithms/ml-labels.md) 与 [`purged_kfold`](../algorithms/ts-cross-validation.md) 的关系 |
| White, "A Reality Check for Data Snooping"（*Econometrica*, 2000） | 论文 / 英 | 随机信号基线与零漂移 GBM 对照的理由 |

## 数据层

对应页面：[barq-data](../crates/barq-data.md)

| 资料 | 类型 / 语言 | 为什么读 |
|---|---|---|
| López de Prado, *AFML* 第 2 章（金融数据结构）、第 11 章（回测的危险） | 书 / 英 | 按时点数据、[前视偏差](../concepts/look-ahead.md) |
| pandas `merge_asof` 文档；kdb+ `aj` 概念 | 文档 / 英 | [as-of join](../algorithms/pit-alignment.md) 的标准语义 |
| Apache Arrow 文档 "Columnar Format"、Arrow C Data Interface：<https://arrow.apache.org/docs/format/CDataInterface.html>；`arrow-rs` 文档 | 文档 / 英 | [`RecordBatch`、投影、`take`](../algorithms/data-loading.md)、零拷贝跨语言 |
| OI Wiki "尺取法"（双指针）；LeetCode "Sliding Window" 标签 | 文档 / 中、英 | `align` 的双指针 |
| PyO3 用户指南 "Parallelism"（`Python::detach`/`attach`） | 文档 / 英 | 解码器为什么每批重新拿 GIL |

## Python 层与门面

对应页面：[barq-py](../crates/barq-py.md) · [barq](../crates/barq.md)

| 资料 | 类型 / 语言 | 为什么读 |
|---|---|---|
| PyO3 用户指南（0.29）：<https://pyo3.rs> | 文档 / 英 | `#[pyclass]`、`Bound`、[GIL](../concepts/portfolio-cache.md)、`abi3` |
| rust-numpy（`numpy` crate）文档 | 文档 / 英 | `PyReadonlyArray`、`to_slice_memory_order` |
| maturin 用户指南；uv 工作区文档 | 文档 / 英 | 构建与安装 |
| NumPy 文档 "Internal memory layout"、"Copies and views" | 文档 / 英 | [F 序](../concepts/python-boundary.md)、strides、`asfortranarray` 何时拷贝 |
| López de Prado, *AFML* 第 7 章；Robert Pardo, *The Evaluation and Optimization of Trading Strategies*（2008）第 11 章 | 书 / 英 | [purged k-fold、walk-forward](../algorithms/ts-cross-validation.md) |
| 《Rust 语言圣经》与 PyO3 中文资料（检索 "PyO3 用户指南 中文"） | 书 / 中 | 入门 |

## 研究协议、验证、基准

对应页面：[研究协议 barq_lab](../tooling/research-protocol.md) · [差分测试与 fixture](../tooling/parity-testing.md) · [基准协议](../tooling/benchmarks.md)

| 资料 | 类型 / 语言 | 为什么读 |
|---|---|---|
| Nosek et al., "The preregistration revolution"（*PNAS*, 2018） | 论文 / 英 | 预注册为什么能防止事后合理化 |
| Harvey, Liu & Zhu, "…and the Cross-Section of Expected Returns"（*RFS*, 2016）；Bonferroni 校正 | 论文 / 英 | [t 统计量门槛](../algorithms/research-gates.md)为什么随试验数上调 |
| Haber & Stornetta, "How to Time-Stamp a Digital Document"（1991） | 论文 / 英 | [哈希链账本](../algorithms/hash-chain-ledger.md) |
| McKeeman, "Differential Testing for Software"（1998）；Jia & Harman, "An Analysis and Survey of the Development of Mutation Testing"（2011） | 论文 / 英 | 差分测试与变异测试的方法论 |
| Mytkowicz et al., "Producing Wrong Data Without Doing Anything Obviously Wrong!"（ASPLOS 2009） | 论文 / 英 | 基准测量的噪声来源；为什么要 best-of-N 和"整机一起慢"检测 |
| Valgrind/callgrind 文档 | 文档 / 英 | 指令数回归门槛（[路线图](../guides/roadmap.md) P1） |
| VectorBT PRO 公开功能页：<https://vectorbt.pro/features/overview/> | 文档 / 英 | [`plan.md`](../../../docs/plan.md) 的功能对照来源（闭源软件，只看文档） |

## 贯穿全书的 Rust 资料

| 资料 | 类型 / 语言 | 用在哪 |
|---|---|---|
| *The Rust Programming Language*（中译《Rust 程序设计语言》，kaisery.github.io/trpl-zh-cn） | 书 / 英、中 | 所有权、借用、trait、生命周期 |
| 《Rust 语言圣经》：<https://course.rs> | 书 / 中 | 同上，更口语化；"性能优化"章对读内核有帮助 |
| Rust 标准库文档：`OnceLock`、`Cow`、`VecDeque`、`partition_point`、`select_nth_unstable_by`、`chunks_mut`、`split_at_mut`、`mem::take` | 文档 / 英 | [记录](../crates/barq-records.md)、[分析](../crates/barq-analysis.md)、[因子引擎](../crates/barq-expr.md)里反复出现 |
| Rust 性能之书 *The Rust Performance Book*（nnethercote） | 书 / 英 | `#[inline(never)]`、寄存器压力、分配 |
| rayon 文档；`cargo doc --workspace --no-deps --open` | 文档 / 英 | 并行；导航 |

## 相关

- [练习与自测](../guides/exercises.md)：同样按领域分节，做题卡住时回到这里找对应的资料。
- [术语表](../reference/glossary.md)：资料里的英文术语（PSR、CSCV、purge、embargo、hash-consing）在这里有一句话的中文定义。
- [核心算法一览](../guides/algorithm-index.md)：每个算法页面末尾的资料都出自这张总表。
- [研究协议 barq_lab](../tooling/research-protocol.md)：预注册与多重比较资料的使用者。
