---
title: Pratt 解析器
kind: algorithm
summary: 一张 (left_bp, right_bp) 绑定力表加一个循环决定所有优先级与结合性：l < min_bp 就停，否则以 right_bp 递归解析右边。
parent: phasor-expr
related: [factor-graph, qlib-operators, look-ahead]
tags: [parser, expressions, compiler]
sources:
  - crates/phasor-expr/src/parse.rs:67-242
order: 701
---

# Pratt 解析器

因子表达式的第一个阶段：把字符串变成 `Expression` 语法树。整个运算符优先级和结合性只由一张绑定力表表达，不需要为每一级优先级写一个递归函数。

## 位置

- `crates/phasor-expr/src/parse.rs:67-242`：入口 `parse` 到解析循环
- `crates/phasor-expr/src/parse.rs:224-232`：绑定力表

## 入口与词法

入口 `parse(source)`：长度 ≤ 16 KiB → `lex` → `expression(min_bp=0, depth=0)` → 必须到达 `End` → `validate`（算子名与元数）。
词法：`$name` 小写化；数字接受 `1e-3`；标识符大小写敏感（`mean(` 在 `validate` 失败）；裸的 `close` 是错误，字段必须写 `$close`。

## 绑定力表

绑定力表（224–232），`(left_bp, right_bp)`：

| 算子 | left | right | 结合 |
|---|---|---|---|
| `\|` | 1 | 2 | 左 |
| `&` | 3 | 4 | 左 |
| `> < >= <= == !=` | 5 | 6 | 左（`a<b<c` = `(a<b)<c`） |
| `+ -` | 7 | 8 | 左 |
| `* /` | 9 | 10 | 左 |
| 前缀 `+ -` | — | 11 | 操作数以 `min_bp = 11` 解析 |
| `**` | 12 | 11 | **右**（left > right） |

## 机制

机制：先解析一个前缀/原子（数字、字段、一元、括号、`Name(args)`），然后循环：看一个运算符，查 `(l, r)`；`l < min_bp` 就停；否则吃掉它，以 `min_bp = r` 递归解析右边，折成 `Binary`。
右结合的 `**`：`a ** b ** c` 里，第一个 `**` 以 11 递归，里面遇到第二个 `**` 的 `l = 12 ≥ 11`，被吸收 → `a ** (b ** c)`。
一元负号的操作数以 11 解析，所以 `-2**2 = -(2**2)`，但 `-a*b = (-a)*b`。
深度 > 128 报错（防 `((((…))))` 爆栈）。`uses_future()` 只检测 `Ref(x, 负字面量)`，够用，因为非字面量偏移在编译期就被拒绝。

这张表与 Python/Qlib 的优先级不同（比较比 `&`/`|` 绑得紧），见 [phasor-expr 的阅读陷阱](../crates/phasor-expr.md#阅读陷阱)第 1 条。

## 相关

- [phasor-expr：因子表达式引擎](../crates/phasor-expr.md)：所属 crate；解析是四个阶段里的第一个。
- [依赖图编译：哈希合并、Ref 视图与槽位调度](../algorithms/factor-graph.md)：下一阶段，把 `Expression` 树编译进共享的依赖图。
- [因子算子与 Qlib 对照](../reference/qlib-operators.md)：解析器接受的字段、运算符与函数一览。
- [延迟与前视偏差](../concepts/look-ahead.md)：`uses_future()` 标出读未来的 `Ref(x, 负数)`。
