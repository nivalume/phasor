---
title: 浮点容差与零钉
kind: concept
summary: is_close 让 NaN 和 ±inf 永远不“接近”，add 把应归零的余额钉成 0；撮合内核的买得起判断离不开它们。
parent: phasor-core
related: [phasor-exec, buy-leg, execute-order-pipeline, parity-testing, conventions]
tags: [floats, tolerance, numerics]
sources:
  - crates/phasor-core/src/math.rs:52-135
  - crates/phasor-exec/src/kernel.rs:144
order: 101
---

# 浮点容差与零钉

金融比较在 phasor 里从不直接用 `==` 或 `<=`。[phasor-core](../crates/phasor-core.md) 的 `math` 模块给出容差比较和余额加法，[phasor-exec](../crates/phasor-exec.md) 的撮合内核是它们最要紧的使用者：差一个舍入误差，一笔订单就可能从成交变成拒单。

## 位置

- `crates/phasor-core/src/math.rs:52-64`：`is_close_tol`
- `crates/phasor-core/src/math.rs:72-84`：`is_close_or_less`、`is_less`
- `crates/phasor-core/src/math.rs:94-135`：`add`、`is_addition_zero`、`sign`
- `crates/phasor-exec/src/kernel.rs:144`：撮合内核的买得起判断

## 容差比较 `is_close`（`math.rs:52-64`）

```text
is_close_tol(a, b, rel_tol=1e-9, abs_tol=1e-12):
    a 或 b 是 NaN           → false
    a 或 b 是 ±inf          → false       ← 在 a == b 之前！
    a == b                  → true
    |a − b| ≤ max(rel_tol × max(|a|, |b|), abs_tol)
```

最后一行就是 PEP 485 的"弱对称相对容差"。
和 Python `math.isclose` 唯一的差别在第二行：`math.isclose(inf, inf)` 是 `True`，这里是 `false`。
原因（文档 13–20 行）：`+inf` 在 phasor 里是活的哨兵——`size = +inf` 表示"能买多少买多少"，`price = +inf` 表示"用收盘价"。两个不同含义的哨兵如果被容差判成"相等"，就是无声的 bug。
vectorbt 的 `utils/math_.py::is_close_nb` 也这么做，所以这是和 PEP 485 的偏差，不是和参考实现的偏差。

派生出两个三值比较（`math.rs:72-84`）：

| 函数 | 定义 | 用途 |
|---|---|---|
| `is_close_or_less(a, b)` | `is_close(a,b) \|\| a < b` | "买得起吗"——差一个舍入误差也算买得起 |
| `is_less(a, b)` | `!is_close(a,b) && a < b` | "真的小于"——容差内算相等，不算小于 |

注意 `is_less(a,b)` **不是** `!is_close_or_less(b,a)`：容差内是第三种状态"相等"。

## 余额更新的零钉 `add`（`math.rs:94-135`）

```text
sign(x):  NaN → i8::MIN；x > 0 → 1；x < 0 → −1；0.0 和 −0.0 → 0       （不用 f64::signum，它把 −0.0 算 −1）
is_addition_zero(a, b):
    sign(a) != sign(b)  → is_close(|a|, |b|)        异号：比较绝对值
    否则                → is_close(a + b, 0.0)       同号：只有两者都≈0 时和才≈0
add(a, b) = is_addition_zero(a, b) ? 0.0 : a + b
```

为什么异号要比绝对值而不是比 `a + b` 和 0：两个几乎相消的大数相加会发生灾难性抵消，`a + b` 的结果相对 0 的距离没有意义，相对容差失效；直接比 `|a|` 和 `|b|` 保住了相对容差。
测试 `addition_snaps_near_cancellation_to_zero`（214 行）：`add(1e9, −(1e9+1)) == 0.0`。
所以 `add` 不是通用加法，只用于现金、持仓、负债、可用现金这些**应该能归零**的余额；[撮合内核](../crates/phasor-exec.md)每一处余额更新都用它。

## 容差为什么在撮合内核里最要命

下面的 `kernel.rs:144` 是 [buy](../algorithms/buy-leg.md) 里的买得起判断。

`kernel.rs:144`：`is_close_or_less(total_req_cash, cash_limit)` 而不是 `<=`。两者只在 `1e-9` 以内不同，
把它换成 `<=` 的变异一开始通过了全部测试（`ARCHITECTURE.zh-CN.md §7.5、§14.4`）。
解决办法有两层：用 `math.nextafter` 构造刚好在边界两侧的 fixture；再用 `size_granularity` 把 `1e-9` 的差异放大成一整手（5.0 对 4.0）。
相反，`buy`/`sell` 里对零的比较（`cash_limit == 0.0`、`adj_size == 0.0`、`position == 0.0`）是**精确**比较（`kernel.rs:43-44`），因为状态在入口已经被零钉过，再用 `is_close` 反而会改变哪些订单成交。

## 相关

- [phasor-core：基础类型](../crates/phasor-core.md)：`math` 模块所在的 crate。
- [phasor-exec：撮合内核](../crates/phasor-exec.md)：仓库里对浮点容差最敏感的地方，每一处余额更新都用 `add`。
- [buy：买入、平空与 lock_cash 预算](../algorithms/buy-leg.md)：`is_close_or_less(total_req_cash, cash_limit)` 所在的买得起判断。
- [execute_order 的固定顺序](../algorithms/execute-order-pipeline.md)：入口先把状态零钉，买卖分支里才能对零做精确比较。
- [差分测试与 fixture](../tooling/parity-testing.md)：边界两侧的 fixture 和变异测试钉住了容差。
- [贯穿全仓库的约定](../guides/conventions.md)：浮点容差层是全仓库的约定之一。
