---
title: from_orders 的一根 K 线
kind: algorithm
summary: simulate_group 每根 K 线先解析全组价格与净值，再按调用顺序逐列执行；value 在 K 线内穿线而不是逐笔重算。
parent: barq-sim
related: [call-seq-auto, cash-flows, execute-order-pipeline, validated-order, parallel-simulation, prepare-order]
tags: [simulation, loop, cash-sharing]
sources:
  - crates/barq-sim/src/from_orders.rs:328-591
  - crates/barq-sim/src/from_orders.rs:254-275
  - crates/barq-sim/src/from_orders.rs:305-312
order: 1001
---

# from_orders 的一根 K 线

`from_orders` 的 `simulate_group` 一次处理一个账户组：每根 K 线先做开盘现金流，再分两遍——第一遍解析全组的价格与估值、算出 `group_value`，第二遍按[调用顺序](../algorithms/call-seq-auto.md)把每列的订单交给[撮合内核](../crates/barq-exec.md)，最后入账收盘收支。它也是 [from_signals 的决策顺序](../algorithms/prepare-order.md)所在循环的模板。

## 位置

- `crates/barq-sim/src/from_orders.rs:328-591`（`simulate_group`）
- `crates/barq-sim/src/from_orders.rs:305-312`（为什么分两遍的注释）
- `crates/barq-sim/src/from_orders.rs:254-275`（`updated_value`）

## 一根 K 线的时序

![一根 K 线](../../../docs/img/zh/bar-loop.svg)

```mermaid
sequenceDiagram
  participant G as simulate_group
  participant C as cash.rs
  participant K as execute_validated
  participant S as Sink
  Note over G: 组初始化：cash = init_cash[0, 首列]；每列 ValidatedOrder 建一次
  loop 每根 K 线 i
    G->>C: at_open：入金（按组首列读）→ 分红（按开盘持仓）
    C->>S: push_cash(Deposit / Dividend)
    Note over G: 第一遍：每列解析 price（+inf→close，−inf→前收）和 val_price（ffill）
    Note over G: group_value = cash + Σ position_k × last_val_price_k
    Note over G: call_seq == Auto：approx_order_value → insert_argsort（NaN 最后）
    loop 按 seq 顺序的每列 k
      G->>K: set_bar(size, price, size_type, direction)；ProcessOrderState{cash, pos_k, debt_k, free_cash, lvp_k, group_value}
      K-->>G: (exec_state, result)
      alt 成交
        G->>S: push_order(i, col, size, price, fees, side)；order_id = fills++
        Note over G: update_value 时 group_value 按 updated_value 重标
      end
      opt log
        G->>S: push_log(LogEntry{前状态, 请求, 后状态, result, order_id})
      end
      Note over G: 写回 cash / free_cash / position_k / debt_k / last_val_price_k
    end
    G->>C: earn：收盘收支，每列
    C->>S: push_cash(Earning)
  end
```

图中的 `ValidatedOrder` 见 [ValidatedOrder：按列提升参数校验](../concepts/validated-order.md)，`execute_validated` 内部的步骤见 [execute_order 的固定顺序](../algorithms/execute-order-pipeline.md)，开盘与收盘的现金流见[成交之外的现金流](../concepts/cash-flows.md)。

## 两遍扫描与穿线的 value

为什么分两遍（`from_orders.rs:305-312`）：组的净值依赖所有列的估值价，而执行顺序可能与布局顺序不同，所以必须先把所有列的价格解析完、算出 `group_value`，再按 `seq` 执行。
`value` 是**穿线**的，不是每笔成交后重算：一根 K 线内所有订单都以开盘时的净值定大小，只有 `update_value=true` 时用 `updated_value`（254–275）按成交价重标：`value' = value + Δcash + (position' × price − position × val_price)`。

## 记录缓冲的容量

预分配：`S::with_capacity(fill_bound)`，`fill_bound` 数"size 非 0 非 NaN 的格子"而不是 `n_rows × n_cols`（100k × 2000 会是 8 GB）。

## 相关

- [barq-sim：模拟循环](../crates/barq-sim.md)：所属 crate，两条循环与四种入口。
- [call_seq=auto：共享账户内的执行顺序](../algorithms/call-seq-auto.md)：第一遍末尾决定第二遍的 `seq`。
- [成交之外的现金流](../concepts/cash-flows.md)：`at_open` 与 `earn` 在一根 K 线里的位置。
- [execute_order 的固定顺序](../algorithms/execute-order-pipeline.md)：第二遍每列调用的内核。
- [按账户组并行与确定性合并](../algorithms/parallel-simulation.md)：多个 `simulate_group` 怎样分到线程上、记录怎样合并。
- [from_signals 的决策顺序](../algorithms/prepare-order.md)：`from_signals` 在同一结构里每列多出的一步 `prepare_order`。
