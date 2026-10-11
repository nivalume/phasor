# 第 12 章 练习与自测

[← 总览](README.md) · [上一章：延伸资料](11-references.md)

每章 3–6 道动手题。原则：**改代码、跑测试、看它怎么失败**，比只读更快建立理解。
所有改动都在本地工作区做，做完 `git checkout -- .` 还原；不要提交练习产生的改动。
前置：`cargo test --workspace` 能通过（约 1–3 分钟，第一次编译更久）。

## 准备

```bash
cargo test --workspace                                   # 全部通过是起点
cargo doc --workspace --no-deps --open                   # 文档导航
cargo run --release --example factor_sweep -p barq       # 端到端一遍
```

## 第 1 章 基础层

1. **容差的边界**：在 `crates/barq-core/src/math.rs` 的测试里加一个断言 `assert!(is_close(1e9, 1e9 + 1.0))` 和 `assert!(!is_close(1e9, 1e9 + 3.0))`，解释为什么 `+1` 在容差内、`+3` 不在（`1e9 × 1e-9 = 1`）。
2. **破坏零钉**：把 `add` 改成普通 `a + b`，跑 `cargo test -p barq-exec --test parity_sequence`。记下第一个失败的 fixture 行和它的 `note` 字段。
3. **列轴**：用 `Level::repeat` / `Level::tile` 构造 3 组参数 × 4 个标的的 `Columns`，打印 `describe(7)` 和 `group_indices("symbol")`，验证第 1 章自测题 3 的答案。
4. **F 序陷阱**：用 `Matrix::from_column_major` 装一个按行主序填的 `Vec`，打印 `column(0)`，亲眼看到转置。

## 第 2 章 撮合内核

1. **手算对照**：`tests/fixtures/execute_order.tsv` 里找一行 `fees > 0 且 fixed_fees > 0 且 size = inf` 的用例，不看代码手算 `out_cash`、`res_size`、`res_fees`，再和 fixture 比对。
2. **换算链**：写一个单元测试：`cash = 1000, position = 20, val_price = 10, value = 1200`，下 `TargetPercent(0.5)` 的订单，预测它会变成多少股的 `Amount` 订单，再用 `execute_order` 验证。
3. **变异**：把 `kernel.rs:144` 的 `is_close_or_less` 换成 `<=`，跑 `cargo test -p barq-exec`。哪条 fixture 抓住了它？去看那一行的 `size_granularity`。
4. **做空记账**：从 100 现金开始，空 5@10，再平 3@12。手算 `cash / position / debt / free_cash` 四个数，用两次 `execute_order` 验证（提示：`avg_entry_price = 50/5 = 10`，`debt_diff = 30`）。
5. **14 种原因**：写一个表，给每种 `StatusInfo` 构造一个最小的 `(state, order)` 触发它。`tests/parity.rs::fixtures_cover_every_reachable_status_info` 可以当答案核对。

## 第 3 章 模拟循环

1. **一根 K 线的顺序**：在 `from_signals.rs::simulate_group` 的入金、`prepare_order`、执行、收盘收支四处各加一行 `eprintln!`，用 `crates/barq-analysis/tests/cash_flows.rs` 里的一个用例跑 `-- --nocapture`，把输出整理成时序图，和 §3.4 对照。
2. **止损先测后 trail**：把 `from_signals.rs` 里 trail 的代码块（595–601 行附近）挪到止损测试之前，跑 `cargo test -p barq-sim a_trailing_stop`，看哪个断言失败、为什么。
3. **call_seq**：构造两列共享账户，第 0 列 `TargetPercent(0.9)`、第 1 列卖出全部，分别用 `CallSeq::Default` 和 `Auto` 跑 `simulate_from_orders`，比较成交。`tests/parity_cash_sharing.rs` 有现成的 fixture 读取代码可以借用。
4. **线程不变性**：把 `random::uniform` 临时改成读一个 `static AtomicU64` 计数器，跑 `cargo test -p barq-sim the_parallel_path`。它为什么失败？
5. **冲突链**：用 `signals.rs` 的四个纯函数手工推演：多头持仓，`long_entry = long_exit = short_entry = true`，`upon_long_conflict = Adjacent`、`upon_dir_conflict = Opposite`、`upon_opposite_entry = Close`、`accumulate = Both`。最终 `SizedOrder` 是什么？写成单元测试验证。
6. **fill_bound**：构造一个每根都有信号的 `FromSignals`，比较 `fill_bound` 的返回值和实际成交数。它为什么是上界而不是精确值？

## 第 4 章 记录

1. **三视图**：买 10@100、卖 4@110、卖 6@90，分别调用 `entry_trades`、`exit_trades`、`positions`，打印每条记录的 `pnl`、`ret`、`status`，验证 Σ pnl 一致而数量不同。
2. **反向成交**：买 10@100、卖 15@110（`fees = 1.5`），看两种视图怎么拆费用（`closed_fees = 10/15 × 1.5 = 1.0`）。
3. **回撤段**：对 `[100, 110, 105, 110, 120, 115, NaN]` 调 `find_drawdowns`，解释为什么最后一段没有记录（末尾 NaN）。去掉 NaN 再跑一次。
4. **order_id 重基**：用两个 `Journal` 各写 2 条日志（`order_id = [0, −1]` 和 `[0, 1]`），`Journal::concat` 后打印 `order_id`。再用 `LogRecords::concat` 做一次，看区别。
5. **ColMap 两条路径**：构造一个乱序的 `col` 数组，`ColMap::build` 后 `ranges()` 是 `None`；用 `MappedArray::sum` 和手工 gather 后求和比较。

## 第 5 章 分析

1. **手工重建**：收盘 `[10, 12, 11]`、`init_cash = 100`、第 0 根买 5@10、第 1 根入金 50。手算 `assets`、`cash`、`value`、`returns`（含入金公式），再用 `Portfolio::from_records` + 一条 `CashRecords` 验证。
2. **+inf Sharpe**：三列，其中一列没有任何成交；调 `sharpe_ratio()` 再 `group_by(Mean)`，看到无穷；加 `traded_only` 再做一次。
3. **PSR 的直觉**：用 `probabilistic_sharpe` 对同一组收益分别传 `benchmark_sr = 0` 和 `= 0.05`，再把收益序列截短一半，观察概率怎么变。
4. **DSR**：生成 1,000 列纯噪声收益（用 `barq_generic::synthetic::random_walk` 的差分），算每列的每根 Sharpe，取最大者做 `deflated_sharpe`，确认它接近 0.5 或更低——"最好的一组参数只是运气"。
5. **PBO**：同上的噪声矩阵传给 `pbo_cscv(n_splits = 8)`，预期约 0.5；再给其中一列加一个真实的正漂移，看 PBO 下降。
6. **统计表**：`Select::with_tags(&[Tag::Trades])` 和 `Select::without_tags(&[Tag::Returns])` 各返回哪些行？对照 §5.6 的表。

## 第 6 章 因子引擎

1. **画图**：对 `["($close - Mean($close,5)) / Std($close,5)", "Ref($close,1)/$close"]` 调 `FactorSet::compile`，在 `graph.rs` 的测试模块里加一个 `#[test]` 打印每个节点的 `op`、`inputs`、`slot`、`view`、`outputs`，画出依赖图并数槽位。
2. **破坏"先分配再释放"**：把 `schedule` 里槽位分配挪到释放之后，跑 `cargo test -p barq-expr`。`slots_are_reused_after_the_last_read` 和哪个 Qlib fixture 会失败？为什么 `run_column` 会读到空 `Vec`？
3. **常量窗口**：对 `[1,1,1,1,1,2]` 算 `Std($x, 5)`，看第 4 行是否精确为 `0.0`；把 `Moments::constant()` 改成永远返回 `false`，再看它变成多少。
4. **ring 与 tree**：把 `RING_MAX` 改成 2，跑 `cargo test -p barq-expr`。全部通过说明什么？再把 `Ordered::kth` 的 `step` 初值改成 `len` 而不是最高 2 的幂，看哪条测试抓住它。
5. **Pratt**：给 `parse.rs` 加测试：`-2**2`、`2**3**2`、`$a>1&$a<3|$b==0`，断言它们的 `Expression` 树形。
6. **性能**：`cargo run --release --example scale -p barq-expr -- 2520 200 sets`，记下 Alpha158 与 Alpha360 的时间和峰值 RSS；把 `evaluate` 里的 `into_par_iter` 换成 `into_iter`（串行）再跑一次，算出加速比。

## 第 7 章 通用算子、指标、信号

1. **min_periods**：对同一列数据比较 `Mean($close, 20)`（barq-expr）和 `ma(close, window=20)`（barq-indicators）的前 20 行。
2. **交叉规则**：对 `a = [0, 1, 1, 2, NaN, 2, 0, 2]`、`b = 1` 手推 `crossed_above_1d(wait=0)` 的输出，再运行验证（答案：第 3 和第 7 根）。
3. **EMA 的两个数值细节**：把 `ewm_mean_1d` 里 `weighted_avg != cur` 的守卫去掉，跑 `cargo test -p barq-generic`，看哪条测试、哪个常量漂了。
4. **三重障碍**：对一条单调上涨的收盘价调 `triple_barrier(up=0.05, down=0.05, max_hold=10)`，看 `decided` 列，解释为什么它就是 `purged_kfold` 需要的 purge 长度。
5. **随机信号**：验证 `random_n(seed, 3)` 是 `random_n(seed, 5)` 的前缀；再验证 `random_entries_exits` 的进出场严格交替。

## 第 8 章 数据层

1. 用 `barq.data.files({...})` 加载 `examples/` 下的一个小 CSV（或自己写三行），调 `prepare()`，确认 `time` 是 `available_time` 的有序并集，缺失格子是 NaN。
2. 构造事件 `(5, 1.0)、(20, 2.0)、(20, NaN)`，面板时间 `[0,10,20,30]`，分别用 `asof`、`window(15, Count)`、`window(15, Mean)` 对齐，核对第 8 章自测题 1。
3. 把 `align` 的 `rows[left].t <= boundary` 改成 `<`，跑 `pytest crates/barq-py/tests/test_data.py`，看窗口变成闭区间后哪个断言失败。

## 第 9 章 Python 层

1. 传一个 C 序 `(100, 3)` 的 `close` 给 `barq._barq.from_signals`（绕过包装器），读错误信息；再用 `barq.from_signals` 传同一个数组，解释为什么没报错。
2. `size = np.array([1., 2., 3.])` 和 `cash_deposits = np.array([1., 2., 3.])` 各自需要多长才合法？构造一个 `n_rows = 3, n_cols = 3` 的运行让两者都合法，再把 `n_cols` 改成 4 看哪个先报错。
3. 用 `@barq.strategy` 写一个双均线策略，`sweep` 4 组参数 × 3 个标的，`cash_sharing=True`，打印 `pf.n_cols`、`pf.group_lens`、`pf.price_nbytes`，和第 9 章自测题 3 对照。
4. `purged_kfold(100, 5, purge=3, embargo=2)` 打印每折的训练集长度，验证第 2 折去掉了 `[37,40)` 和 `[60,62)`。
5. 把 `walk_forward` 改成记录所有测试分数并取最大，写一个测试证明它会"看见未来"。

## 第 10 章 研究协议、验证、基准

1. 在临时目录初始化一个 `Ledger`，追加两条，手工编辑 `ledger.jsonl` 的第一条，再打开它，看到 `LedgerError`。
2. 用 `policy.t_required` 算 N = 1, 10, 50, 200 的双侧门槛，画一条曲线，理解为什么研究协议"大多数时候在拒绝"。
3. 写一个故意前视的表达式（`Ref($close, -1)`）和一个隐蔽前视的（例如在 `harness` 之外用 `bshift`），分别看 `uses_future` 和 `contract_check` 哪个抓得住。
4. 跑 `cargo run --release --example bench -p barq-sim` 两次，比较 `best_secs` 的差异，体会为什么门槛是 10%、为什么只看单线程行。
5. 挑 `tests/parity/README.md` 变异表里的一条"幸存者"，读它的论证，判断你是否同意它"不可观测"。

## 综合题（读完全部章节后）

1. **跟一笔交易走完全程**：从 `examples/sweep.py` 的某个参数组里选一个标的，找到它第一笔成交：它由哪个 `Mean` 节点驱动（第 6 章）→ 哪根 K 线产生进场信号、延迟到哪根（第 9 章）→ `prepare_order` 怎么把它变成 `Pending`（第 3 章）→ `buy` 里成交了多少股、付了多少费（第 2 章）→ 它在 `OrderRecords` 的哪一行、在 `exit_trades` 里配对了哪笔出场（第 4 章）→ 它对 `returns` 的贡献（第 5 章）。
2. **加一个算子**：给 `barq-expr` 加 `Sqrt`（提示：`UnaryOp` + `unary()` + `validate` + Polars 后端 + 测试），跑通全部测试。这是路线图 §17.2 列出的工作之一。
3. **加一种现金流**：给 `CashFlowKind` 加 `Interest`，在 `cash.rs` 里按 `debt_rate` 每根计息，并让 `series.rs` 的重建链通过 `cash_flows.rs` 的守恒检查。这是 `plan.md` C4 的一部分；做完你就理解了"记录是唯一来源"的全部含义。
