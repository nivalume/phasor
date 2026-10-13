---
title: 时间序列交叉验证
kind: algorithm
summary: rolling、expanding、purged_kfold 按时间切分行号，walk_forward 只凭训练分数选参，只记录被选参数的测试分数。
parent: barq-py
related: [ml-labels, overfitting-stats, research-harness, look-ahead, strategy-sweep]
tags: [validation, python, overfitting]
sources:
  - crates/barq-py/python/barq/validation.py
  - crates/barq-py/tests/test_validation.py
order: 1304
---

# 时间序列交叉验证

`barq.validation` 是纯 NumPy 的模块：它只产出行号切分（`Split(train, test)`），再用 `walk_forward` 把"在训练段选参数、在测试段打分"串起来。它不调用 Rust，所以 `test_validation.py` 按路径加载、不需要 Rust 构建。

## 位置

- `crates/barq-py/python/barq/validation.py:83`（`rolling`）
- `crates/barq-py/python/barq/validation.py:130`（`expanding`）
- `crates/barq-py/python/barq/validation.py:174`（`purged_kfold`）
- `crates/barq-py/python/barq/validation.py:253`（`walk_forward`）

## 四个函数

```text
rolling(n, train, test, step=test):   s = 0, step, …；train = [s, s+train)，test = [s+train, s+train+test)；装不下整窗的尾巴丢掉
expanding(n, min_train, test, step):   e = min_train, …；train = [0, e)，test = [e, e+test)
purged_kfold(n, k, purge=0, embargo=0):
    folds = array_split(arange(n), k)（前 n % k 个多一行）
    对每个 fold [start, stop)：keep = rows < start − purge  |  rows ≥ stop + embargo
    train = rows[keep]，test = fold
    ← purge 去掉标签向前看会碰到测试段的训练行；embargo 去掉特征向后看会碰到测试段的训练行（AFML 第 7 章）
walk_forward(fn, n, splits, *, select=argmax):
    对每个 split：校验非空、在 [0,n) 内、不相交
    train_scores = fn(split.train)；chosen = select(train_scores)（NaN 永不胜出，平局取 fn 顺序的第一个）
    test_scores = fn(split.test)；只记录 test_scores[chosen]，其余测试分数丢弃——它们不能影响选择
```

## purge 与标签的决定行

[机器学习标签](../algorithms/ml-labels.md)里 `triple_barrier` 返回的"决定行"就是 `purge` 的依据：标签在 `t` 的决定发生在 `t + h`，训练集里 `[start − h, start)` 的行会泄露测试段。

研究层 `harness.composite` 里另有一套 walk-forward，与这里的 `walk_forward` 是两套实现，见 [评估框架与合同检查](../algorithms/research-harness.md)。

## 相关

- [barq-py：Python 层](../crates/barq-py.md)：`barq.validation` 是 Python 包的一部分。
- [机器学习标签](../algorithms/ml-labels.md)：`triple_barrier` 的决定行决定 `purge` 取多大。
- [过拟合检验：PSR、DSR、PBO](../algorithms/overfitting-stats.md)：同一个"选中的参数被高估"问题的统计量一侧。
- [评估框架与合同检查](../algorithms/research-harness.md)：研究层自己的 walk-forward 组合。
- [延迟与前视偏差](../concepts/look-ahead.md)：purge 与 embargo 防的是哪种泄露。
- [@barq.strategy 的参数扫描](../algorithms/strategy-sweep.md)：`fn` 里通常跑的就是一次扫描。
