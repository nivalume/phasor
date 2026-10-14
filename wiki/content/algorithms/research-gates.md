---
title: 门槛、多重比较与原创性检查
kind: algorithm
summary: Bonferroni 让 t 门槛随账本里的测试数 N 上调，负对照必须被拒绝，最大公共子树判定换价格字段或重扫参数不算新因子。
parent: research-protocol
related: [overfitting-stats, hash-chain-ledger, research-harness, factor-graph]
tags: [research, statistics, multiple-testing]
sources:
  - research/phasor_lab/protocol/policy.py:64-68
  - research/phasor_lab/types/factor_ts/__init__.py:96-136
  - research/phasor_lab/types/factor_ts/spec.py:212-249
  - research/phasor_lab/types/factor_ts/calibration.py
order: 103
---

# 门槛、多重比较与原创性检查

[研究协议](../tooling/research-protocol.md)对每份规格只判一次，判据全部写死在 `policy.py` 里：t 门槛随[账本](../algorithms/hash-chain-ledger.md)里已做过的测试数上调，另有几条稳健性条件；`calibrate` 用负对照证明这些门槛真的会拒绝；原创性检查防止把同一个结构换个价格字段再测一次。

## 位置

- `research/phasor_lab/protocol/policy.py:64-68`（`t_required`）
- `research/phasor_lab/types/factor_ts/__init__.py:115-136`（样本内 gate 与 OOS gate）
- `research/phasor_lab/types/factor_ts/__init__.py:96-109`（跨资产）
- `research/phasor_lab/types/factor_ts/spec.py:212-249`（`common_subtree`）
- `research/phasor_lab/types/factor_ts/calibration.py`（负对照）

## 门槛与多重比较

```text
t_required(N, α, two_sided) = Φ⁻¹(1 − α/(2N))（双侧）或 Φ⁻¹(1 − α/N)（单侧）     ← Bonferroni
    α = 0.05：N=1 → 1.96，N=10 → 2.81，N=50 → 3.29
样本内 gate（全部必须通过）：
    net_t_stat ≥ Φ⁻¹(1 − α/2N)，N = 账本里的样本内测试数 + 1（运行时固定为 tests_at_run）
    not_suspicious：t ≤ 8（更高视为前视）
    sharpe_at_2x_cost：2× 成本下 sharpe > 0
    grid_members_positive：成员 sharpe > 0 的比例 ≥ 0.6
    cross_asset_positive：单资产 sharpe > 0 的比例 ≥ 0.75
OOS gate：单侧 Bonferroni（M = 已用 OOS 测试数 + 本批候选数）、not_suspicious、sharpe_2x > 0
forward：z = (mean(daily_fwd) − daily_mean_IS) / (daily_std_IS / √n_fwd)；z < −2 → retired；≥ 180 天按 sharpe > 0 定 final
```

## 负对照

`calibrate` 跑负对照：leak 规格必须被 regulator 拒绝（负 `Ref`）；random/shuffle/leak 对照以最松的 `tests=1` 运行且必须被拒绝，leak 必须触发 `not_suspicious`。

## 原创性检查

**原创性检查**（`spec.py:212-249`）：`common_subtree` 是最大自顶向下公共子树（数字和 `{param}` 是可互换的自由节点，可选地统一所有价格字段，交换律算子两种顺序都试）；`same_structure ⇔ size(a) == size(b) == common_subtree(a, b, unify_prices=True)`，所以把 `$close` 换成 `$vwap` 或重新扫参数不算新东西。

## 相关

- [研究协议 phasor_lab](../tooling/research-protocol.md)：门槛在状态机的 `run`、`oos`、`forward` 三处判定。
- [过拟合检验：PSR、DSR、PBO](../algorithms/overfitting-stats.md)：DSR 与 PBO 从另一面处理"试了很多次才找到一个好的"。
- [哈希链账本](../algorithms/hash-chain-ledger.md)：N 和 M 从账本里数出来，所以失败的运行也抬高下一次的门槛。
- [评估框架与合同检查](../algorithms/research-harness.md)：t、sharpe、成员与跨资产比例在这里算出。
- [依赖图编译：哈希合并、Ref 视图与槽位调度](../algorithms/factor-graph.md)：引擎里另一种"结构相同即同一节点"的判定。
