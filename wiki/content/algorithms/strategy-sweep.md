---
title: "@phasor.strategy 的参数扫描"
kind: algorithm
summary: 每组参数调一次策略函数、延迟信号、横向拼接，column_map 把逻辑列 p×S+s 映回源列 s，价格从不平铺。
parent: phasor-py
related: [signal-sweep, crossover, look-ahead, flex-array, python-boundary]
tags: [python, sweep, signals]
sources:
  - crates/phasor-py/python/phasor/strategy.py:79-149
  - crates/phasor-py/python/phasor/strategy.py:36-76
order: 1303
---

# @phasor.strategy 的参数扫描

`@phasor.strategy` 把一个"给数据和参数、返回 `(entries, exits)`"的 Python 函数变成一次扫描：每组参数调一次函数，信号延迟后横向拼成 `参数 × 标的` 列，一次 `from_signals` 跑完。Rust 侧对应的是 [SignalSweep](../algorithms/signal-sweep.md)，两者的列号规则相同：参数优先，标的其次。

## 位置

- `crates/phasor-py/python/phasor/strategy.py:79-149`（`strategy`，内含 `sweep` 与 `run`）
- `crates/phasor-py/python/phasor/strategy.py:36-76`（`factors`、`crossed_above`、`_signal`、`_delay`）

## sweep 的步骤

```text
sweep(data, *, params, signal_delay=1, **options):
    ① signal_delay 非负整数（拒绝 bool）；options 里不许出现 close/entries/exits/open/high/low/column_map/group_lens
    ② close = asarray(data["close"])，1-D → (T,1)
    ③ 对每组参数：signature.bind(data, **group)（未知参数在跑之前就是 TypeError）→ 调函数 → 必须返回 (entries, exits)
       → _signal（bool，形状 == close.shape）→ _delay(bars)：delayed[bars:] = array[:-bars]，顶部补 False
    ④ market = data 里有的 open/high/low
    ⑤ cash_sharing 时 group_lens = [S] × n_params（每组参数一个账户）
    ⑥ from_signals(close, concatenate(entries, axis=1), concatenate(exits, axis=1), column_map = tile(arange(S), n_params), **market, **options)
    结果列 = p × S + s（参数优先，标的其次）
```

## factors、crossed_above 与延迟

`factors(data, **exprs)` 编译 `FactorSet` 并拒绝 `uses_future`；`crossed_above` 走引擎的相等/NaN 规则（见 [crossed_above 的交叉规则](../algorithms/crossover.md)）。`_delay(1)` 配合默认 `price = +inf`（本根收盘）意味着"在 `t` 决定、在 `t+1` 的收盘成交"。`tile(arange(S), P)` 把逻辑列 `p×S+s` 映射到源列 `s`，价格永远不被平铺（`Portfolio.price_nbytes == panel.close.nbytes`）。

## 相关

- [phasor-py：Python 层](../crates/phasor-py.md)：`strategy.py` 是 Python 包的一部分。
- [SignalSweep：因子到信号列](../algorithms/signal-sweep.md)：Rust 侧的同一种扫描，`out_col = case_idx × n_symbols + symbol`。
- [crossed_above 的交叉规则](../algorithms/crossover.md)：`phasor.crossed_above` 调的就是它。
- [延迟与前视偏差](../concepts/look-ahead.md)：`signal_delay=1` 与 `uses_future` 各管哪一半。
- [FlexArray 广播与列映射](../concepts/flex-array.md)：`column_map` 怎么让一份源宽度的价格服务全部参数列。
- [Python 边界：F 序与一维的三种含义](../concepts/python-boundary.md)：拼好的数组进入 Rust 时的检查。
