---
title: 评估框架与合同检查
kind: algorithm
summary: 一次 barq 调用算因子与 z 分数，按 card 方向生成仓位，一次 from_orders 回测全部成本倍数；合同检查用前缀一致抓前视。
parent: research-protocol
related: [look-ahead, ts-cross-validation, research-gates, factor-graph, call-seq-auto, flex-array]
tags: [research, backtest, look-ahead]
sources:
  - research/barq_lab/types/factor_ts/harness.py
  - research/tests/test_harness.py:81-96
order: 102
---

# 评估框架与合同检查

`harness.py` 是[研究协议](../tooling/research-protocol.md)里真正调用 barq 的地方：它把一份已批准的规格变成因子值、仓位、目标权重和一次批量回测，再算出[门槛](../algorithms/research-gates.md)要看的统计量。批准之前，`contract_check` 先在合成数据上证明这份规格既不偷看未来、也真的能跑通。

## 位置

- `research/barq_lab/types/factor_ts/harness.py:29-39`（`evaluate`）
- `research/barq_lab/types/factor_ts/harness.py:42-51`（`zscore`）
- `research/barq_lab/types/factor_ts/harness.py:64-83`（`position`）
- `research/barq_lab/types/factor_ts/harness.py:143-155`（`targets`）
- `research/barq_lab/types/factor_ts/harness.py:158-190`（`backtest`）
- `research/barq_lab/types/factor_ts/harness.py:336-353`（`composite`）
- `research/barq_lab/types/factor_ts/harness.py:429-454`（`contract_check`）
- `research/tests/test_harness.py:81-96`（`composite` 的因果性断言）

## 评估流程

```text
evaluate：FactorSet.compile(expressions)；uses_future → 报错；非有限 → NaN
zscore：一次 barq 调用算 ($x − Mean($x,w)) / Std($x,w)，前 w 行置 NaN（因果）
position：sign → sign(direction × factor)
          zscore → clip(direction × z / scale, −1, 1)
          band → 多头状态 = ffill({z > entry: 1, z < exit: 0})，空头对称；side 裁剪到 [0,1] 或 [−1,0]
          card 的 direction 由框架施加，从不搜索
members：spec.expand 的网格；相同表达式只算一次；负对照用 random（N(0,1)）/ shuffle（720 根块置换）/ leak（close[t+25]/close[t+1] − 1）替换
composite：ensemble → mean(positions)；walk_forward → 每个成员 1× 成本回测，按 lookback 窗口的 sharpe/mean_return 选 argmax，
           每 refit 根重选，前 lookback 根空仓（因果性由 test_harness.py:81-96 断言）
targets：weights = round(positions/step)·step / symbols；只在变化时下单（否则 NaN，持仓漂移）；out[1:] = orders[:-1]（t 决定，t+1 执行）
backtest：所有 (targets × 成本倍数) 块横向拼接，一次 from_orders(size_type="targetpercent", cash_sharing=True,
          group_lens=[symbols]×groups, call_seq="auto", column_map=tile(arange(symbols), groups), ann_factor=8760, threads=4)
stats：日收益取每个 UTC 日最后一根；t = mean/std(ddof=1)·√days；sharpe = mean(hourly)/std·√8760；max_drawdown = min(values/cummax − 1)
```

`backtest` 用到的引擎机制：`targetpercent` 的换算见 [数量换算链与 percent](../algorithms/size-cascade.md)，共享账户内先卖后买见 [call_seq=auto](../algorithms/call-seq-auto.md)，`column_map` 见 [FlexArray 广播与列映射](../concepts/flex-array.md)。`composite` 的 walk-forward 与 `barq.validation.walk_forward` 是两套实现，后者见 [时间序列交叉验证](../algorithms/ts-cross-validation.md)。

## 合同检查

**合同检查**（`contract_check`）：800 根合成 GBM 面板，`cut = 600`；每个表达式在全长和前 600 行上各算一次；拒绝"后 600 行里非有限比例 > 50%"、"常量"、以及 **`!allclose(前缀结果, 全长结果[:600])`——加了后面的 K 线会改变前面的值 = 前视**。然后跑一遍完整链路证明能执行。

## 相关

- [研究协议 barq_lab](../tooling/research-protocol.md)：`approve` 调 `contract_check`，`run`/`oos`/`forward` 调评估流程。
- [延迟与前视偏差](../concepts/look-ahead.md)：`uses_future`、因果 z 分数、`t+1` 执行与前缀一致检查各防一种前视。
- [门槛、多重比较与原创性检查](../algorithms/research-gates.md)：这里算出的 t、sharpe 与成员比例交给门槛判定。
- [时间序列交叉验证](../algorithms/ts-cross-validation.md)：引擎 Python 包里的另一套 walk-forward。
- [依赖图编译：哈希合并、Ref 视图与槽位调度](../algorithms/factor-graph.md)：`FactorSet.compile` 把相同子表达式只算一次。
- [call_seq=auto：共享账户内的执行顺序](../algorithms/call-seq-auto.md)：再平衡时先卖后买，释放现金。
