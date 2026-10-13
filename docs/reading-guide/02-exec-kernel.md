# 第 2 章 撮合内核：`barq-exec`

[← 总览](README.md) · [上一章：基础层](01-foundations.md) · [下一章：模拟循环](03-sim.md)

## 2.1 定位

`barq-exec` 只做一件事：**一笔订单，对一个账户，在一根 K 线上，算出成交或不成交**。
它不知道时间轴、不知道别的列、不知道止损和信号；这些都在 `barq-sim`。
把它单独拆出来的原因写在 `crates/barq-exec/src/lib.rs:3-10`：向量化层和将来的事件驱动层都调用同一个 `execute_order`，
所以两层的结果才可以逐列互相验证。

读这个 crate 的收获：理解 vectorbt `execute_order_nb` 的全部语义，包括 14 种拒单原因、做空的保证金记账和费用反解。
它也是仓库里**对浮点容差最敏感**的地方（§2.6）。

## 2.2 文件与阅读顺序

| 顺序 | 文件 | 行数 | 读什么 |
|---|---|---:|---|
| 1 | `src/lib.rs` | 53 | 模块文档：两种失败、确定性、验证策略 |
| 2 | `src/types.rs` | 190 | 四个类型；重点是 `ExecuteOrderState` 的 `debt` / `free_cash` 注释（`types.rs:5-17`）和 `Order::default` 里的哨兵值（`types.rs:118-139`） |
| 3 | `src/kernel.rs:528-701` | — | `execute_order` → `execute_validated`：零钉、忽略检查、校验、换算链、随机拒单、分发。先读这一段再读买卖 |
| 4 | `src/kernel.rs:41-221` | — | `buy`：现金预算、费用反解、同一笔成交里如何平空 |
| 5 | `src/kernel.rs:223-379` | — | `sell`：可卖数量、开空时如何记 `debt` |
| 6 | `src/kernel.rs:381-526` | — | `validate_bar` / `param_violation` / `ValidatedOrder`（性能优化，可最后读） |
| 7 | `tests/parity.rs`、`tests/parity_sequence.rs` | — | 看 fixture 怎么被重放、`agree` 怎么比较 |
| 8 | `tests/fixtures/execute_order.tsv` | 474 行 | 挑 `buy_plain`、`*fees*`、`*granularity*`、`*short*` 几行手算 |

第一遍可以跳过 `ValidatedOrder`，把 `execute_validated` 当 `execute_order` 读。

## 2.3 类图

```mermaid
classDiagram
  class ExecuteOrderState {
    +f64 cash
    +f64 position  （有符号，负为空头）
    +f64 debt      （空头开仓所得，按平均入场价）
    +f64 free_cash （扣掉空头占用后的现金）
  }
  class ProcessOrderState {
    +cash, position, debt, free_cash
    +f64 val_price （估值价）
    +f64 value     （账户总净值，共享账户时是整组）
    +usize oidx, lidx
    +flat(cash, val_price)$
  }
  class Order {
    +f64 size = +inf
    +f64 price = +inf
    +SizeType size_type = Amount
    +Direction direction = Both
    +f64 fees, fixed_fees, slippage = 0
    +f64 min_size = 0, max_size = +inf
    +f64 size_granularity = NaN
    +f64 cash_limit = +inf
    +f64 reject_prob = 0
    +bool lock_cash, allow_partial=true, raise_reject, log
  }
  class ValidatedOrder {
    -Order order
    -Option~&'static str~ violation
    +new(Order)$
    +set_bar(size, price, size_type, direction)
    +check() Result
  }
  class OrderResult {
    +f64 size, price, fees （未成交为 NaN）
    +Option~OrderSide~ side
    +OrderStatus status
    +Option~StatusInfo~ status_info
    +not_filled(status, info)$
    +filled(size, price, fees, side)$
  }
  class SizeType { <<enum>> Amount; Value; Percent; TargetAmount; TargetValue; TargetPercent }
  class Direction { <<enum>> LongOnly; ShortOnly; Both }
  class OrderStatus { <<enum>> Filled; Ignored; Rejected }
  class StatusInfo { <<enum>> 14 种原因 }

  ProcessOrderState ..> ExecuteOrderState : 前 4 个字段相同
  ValidatedOrder o-- Order
  Order --> SizeType
  Order --> Direction
  OrderResult --> OrderStatus
  OrderResult --> StatusInfo
```

`execute_order(&ProcessOrderState, &Order, Option<f64>) -> Result<(ExecuteOrderState, OrderResult)>` 是唯一入口；
`execute_validated` 是跳过参数校验的同义入口（`kernel.rs:559`）。

## 2.4 核心算法一：`execute_validated` 的固定顺序

位置：`kernel.rs:559-701`。顺序不能换，每一步都有 fixture 钉住。

```mermaid
flowchart TD
  A["零钉：cash/position/debt/free_cash/val_price/value<br/>is_close(v,0) → 0"] --> B{"size 是 NaN？"}
  B -- 是 --> I1["Ignored · SizeNaN"]
  B -- 否 --> C{"price 是 NaN？"}
  C -- 是 --> I2["Ignored · PriceNaN"]
  C -- 否 --> D["validate_bar（7 项：现金、持仓、负债、price>0、方向与持仓一致）"]
  D --> E["validated.check()（7 项参数：fees 有限、slippage≥0、…）"]
  E --> F{"reject_prob>0 且没给 draw？"}
  F -- 是 --> ERR["Err(Invariant)"]
  F -- 否 --> G["数量换算链（§2.5）"]
  G --> H{"draw < reject_prob？"}
  H -- 是 --> R["Rejected · RandomEvent，状态不变"]
  H -- 否 --> J{"order_size > 0？"}
  J -- 是 --> BUY["buy()"]
  J -- "否（0 和 NaN 也走这边）" --> SELL["sell(-order_size)"]
```

三个容易漏掉的点：

1. **忽略检查在校验之前**（`kernel.rs:583-596`）。一列全是 NaN 的 size 配上非法的 fees，结果是整列 Ignored 而不是报错。这正是 `ValidatedOrder` 把违例"带着走"而不是构造时就报错的原因（`kernel.rs:468-474`）。
2. **先 bar 后参数**（`kernel.rs:598-602`）。两组校验独立，但一个订单同时违反两边时报哪条消息由顺序决定，测试 `a_bad_state_outranks_a_bad_parameter` 钉住。
3. **随机拒单在分发之前**（`kernel.rs:679-693`）。参考实现是在买卖之后覆盖结果；这里提前决定，省掉回滚。唯一的可观测差异：被随机拒掉的订单不会再触发"无限做多/做空"那两个 `Err`。

## 2.5 核心算法二：数量换算链

位置：`kernel.rs:610-677`。每一步都把 `size_type` 改写成更"低级"的类型，最终只有 `Amount` 到达买卖分支。

![换算链](../img/zh/cascade.svg)

| 步骤 | 条件 | 变换 | 可能的提前返回 |
|---|---|---|---|
| 0 | `direction == ShortOnly` | `size = -size`（空头列里正数表示"再空一些"） | — |
| 1 | `TargetPercent` | `size *= value`；类型 → `TargetValue` | `value` NaN → Ignored·ValueNaN；`value <= 0` → Rejected·ValueZeroNeg |
| 2 | `Value` 或 `TargetValue` | `size /= val_price`；`Value` → `Amount`，`TargetValue` → `TargetAmount` | `val_price` 无限或 ≤ 0 → Err；NaN → Ignored·ValPriceNaN |
| 3 | `TargetAmount` | `size -= position`；→ `Amount` | — |
| 4 | `Amount` 且可做空且 `size == -inf` | `size = -1`；→ `Percent`（"空掉全部资源"） | — |
| 5 | `Percent` | `percent = |size|`；`size = sign(size) × inf` | `sign(0) × inf = NaN`，后面走 sell 分支并以 SizeZero 被忽略 |

注意 `sign` 是 NumPy 语义（`kernel.rs:25-33`）：`sign(NaN) = 0`，所以 `0 × inf = NaN` 和参考实现一致。

**`percent` 是可用资源的比例，不是目标仓位。** 只有上表第 5 步会把它设成 `|size|`；其它类型保持 `NaN`，`buy` / `sell` 都不看它。`TargetPercent` 在第 1 步已经乘进组合净值，后面按目标股数和当前持仓的差额成交。

```mermaid
flowchart LR
  A["Percent 正数"] --> B["buy：现金预算 × percent"]
  C["Percent 负数，或 size = −inf"] --> D["sell：可卖数量 × percent"]
  E["TargetPercent"] --> F["先变成目标股数差额<br/>percent 保持 NaN"]
  classDef base fill:#ddf4ff,stroke:#0969da;
  classDef part fill:#fff8c5,stroke:#9a6700;
  class B,D base;
  class F part;
```

正比例进 `buy`（`kernel.rs:89-91`）。乘法发生在预算已经和 `cash`、`order.cash_limit`、以及 `lock_cash` 算出的上限取过最小之后。现金 100、`cash_limit = 60`、`percent = 0.5` 时最多花 30。此时请求量是 `+inf`，股数由这笔钱反推。

负比例进 `sell`（`kernel.rs:268-291`）。`LongOnly` 的基数是当前持仓。允许做空时，基数是卖掉多头之后 `free_cash` 还能新开的空头。`lock_cash` 为真时，比例在这个上限里面乘掉，随后把 `percent` 置成 `NaN`，避免函数后面再乘一次。`size = −inf` 被第 4 步改写成 `Percent(−1)`，也就是把这份资源全部拿去卖或开空。

## 2.6 核心算法三：`buy`

位置：`kernel.rs:48-221`。输入 `size > 0`（可为 `+inf`）、`percent`（NaN 表示没有）。

```text
adj_price = price × (1 + slippage)

cash_limit =
  if lock_cash:
     position ≥ 0 → free_cash
     position < 0 → 先算"平掉空头要多少钱" cover_req；
                    够平且有余 → free_cash + 2·debt
                    不够平     → 解出最多能平多少（见下）
                    恰好够     → cash
  else: cash
cash_limit = min(cash_limit, cash, order.cash_limit)        # cash_limit 是 barq 新增，参考实现没有
if percent 给定: cash_limit = min(cash_limit, percent × cash_limit)

LongOnly/Both:  cash_limit == 0 → Rejected·NoCashLong
                size 与 cash_limit 都是 inf → Err（无限做多）
ShortOnly:      position == 0 → Rejected·NoOpenPosition   # 只能平空，不能开多

adj_size = ShortOnly ? min(-position, size) : size
adj_size == 0            → Ignored·SizeZero
adj_size > max_size      → allow_partial ? adj_size = max_size : Rejected·MaxSizeExceeded
有粒度                    → adj_size = floor(adj_size / g) × g

req_cash  = adj_size × adj_price
req_fees  = req_cash × fees + fixed_fees
total     = req_cash + req_fees

if is_close_or_less(total, cash_limit):                      # 买得起
    final = adj_size, fees_paid = req_fees, spend = total
else:                                                        # 买不起：反解
    max_req_cash = add(cash_limit, -fixed_fees) / (1 + fees)
    max_req_cash ≤ 0 → Rejected·CantCoverFees
    max_acq_size = max_req_cash / adj_price
    无粒度 → final = max_acq_size, fees_paid = cash_limit − max_req_cash, spend = cash_limit
    有粒度 → final = floor(max_acq_size / g) × g，重算 req_cash 与 fees   # 粒度第二次生效

is_close(adj_size, 0)                 → Ignored·SizeZero      # 注意测的是 adj_size 不是 final
is_less(final, min_size)              → Rejected·MinSizeNotReached
size 有限 且 final < size 且 !allow_partial → Rejected·PartialFill

new_cash     = add(cash, -spend)
new_position = add(position, final)
position < 0（在平空）：
    short_size = new_position < 0 ? final : |position|
    avg_entry  = debt / |position|
    debt_diff  = short_size × avg_entry
    new_debt      = add(debt, -debt_diff)
    new_free_cash = add(free_cash + 2·debt_diff, -spend)      # 释放的保证金是 2 倍
否则：new_free_cash = add(free_cash, -spend)
```

**平空就在这一笔买单里。** 同一列只有一个带符号的持仓，`new_position = add(position, final_size)`（`kernel.rs:193`）。函数只返回一个 `OrderResult::filled(..., Buy)`（`kernel.rs:212-219`）；模拟循环对它 `push_order` 一次（`crates/barq-sim/src/from_orders.rs:520-522`）。`short_size` 只用来减 `debt`，不再执行第二张订单。

```mermaid
flowchart TD
  A["一笔 Buy，数量 final_size"] --> B["new_position = position + final_size"]
  B --> C{"原 position < 0？"}
  C -- 否 --> D["debt 不变<br/>free_cash -= spend"]
  C -- 是 --> E{"new_position < 0？"}
  E -- 是 --> F["short_size = final_size<br/>空头还在"]
  E -- 否 --> G["short_size = abs(position)<br/>多出的股数是新多头"]
  F --> H["释放 debt，free_cash 加回两倍"]
  G --> H
  classDef base fill:#ddf4ff,stroke:#0969da;
  classDef hot fill:#ffebe9,stroke:#cf222e;
  class A base;
  class H hot;
```

持仓 −5、买得起 8 股：订单记录是买 8，持仓变成 3，`short_size = 5`。买 3 股则持仓变成 −2，`short_size = 3`，没有新多头。`ShortOnly` 在更前面就把数量截成不超过当前空头（`kernel.rs:113-117`），所以走不到"买穿"。统计上要把这一笔拆成"已平的空头"和"新多头的开仓"，是第 4 章的 `exit_trades`（`crates/barq-records/src/trades.rs:580-606`）：它立刻写下已平的那一段，剩下的股数要等以后的卖出或收盘标记才另成一条记录。

**费用反解**为什么是 `(cash_limit − fixed_fees) / (1 + fees)`：费用在上限*之内*。设可花名义金额 `x`，则 `x + x·fees + fixed = cash_limit`，解得 `x = (cash_limit − fixed)/(1 + fees)`。
例：现金 100、`fees=0.1`、`fixed_fees=1`、`price=10`、`size=+inf` → `x = 99/1.1 = 90`，买 9 股，付费 10，现金归零。这就是总览自测题的答案。

**`lock_cash` 决定这笔单能不能动用空头保证金。** 保证金本身在 `sell` 里记账，与这个开关无关（§2.7）。开关打开时，买单的预算从 `free_cash` 起算，而不是从 `cash` 起算（`kernel.rs:58-91`）。当前是空头时，平仓会把这一列的两份预留还回来，所以"够平"的预算是 `free_cash + 2·debt`，再拿去和整笔平仓成本比。这三路比较是精确比较。

```mermaid
flowchart TD
  Q{"lock_cash？"} -- 否 --> C["预算从 cash 起"]
  Q -- 是 --> P{"position ≥ 0？"}
  P -- 是 --> F["预算从 free_cash 起"]
  P -- 否 --> R{"free_cash + 2×debt 与平仓成本"}
  R -- 大于 --> A["预算 = free_cash + 2×debt<br/>平完后剩余的部分可开多"]
  R -- 小于 --> B["反解最多能平的股数<br/>预算只覆盖这一段"]
  R -- 等于 --> E["预算从 cash 起<br/>恰好平完"]
  C --> M["再与 cash、order.cash_limit 取最小<br/>percent 有值再乘"]
  F --> M
  A --> M
  B --> M
  E --> M
  classDef base fill:#ddf4ff,stroke:#0969da;
  classDef hot fill:#ffebe9,stroke:#cf222e;
  class A,B hot;
  class M base;
```

沿用 §2.7 的账户：现金 100 空 5 股 @10 之后，`cash = 150`、`debt = 50`、`free_cash = 50`，于是 `free_cash + 2·debt = 150`。其中 50 用来平空，剩下 100 可以开多。单列、没有其他空头时，`free_cash + 2·debt` 等于当时的 `cash`。同组里别的列也有空头时，组现金里还含着它们的 `2·debt`，这一列只加回自己的一份（共享方式见第 3 章）。

不够平完时（`kernel.rs:72-79`）：设最多能平 `s` 股，花费 `s·p·(1+fees) + fixed`，每平 1 股释放 `2·avg_entry`，于是
`free_cash + 2·avg_entry·s = s·p·(1+fees) + fixed` → `s = (free_cash − fixed) / (p(1+fees) − 2·avg_entry)`。
解出来的 `s` 小于整笔空头，这笔买单不会开多。

## 2.7 核心算法四：`sell`

位置：`kernel.rs:228-379`。输入 `size > 0`。

```text
adj_price = price × (1 − slippage)

size_limit =
  LongOnly                         → min(position, size)                # 只能卖持有的
  lock_cash 或 (size=inf 且有 percent) →
      long_size  = max(position, 0)
      long_cash  = long_size × adj_price × (1 − fees)                  # 卖多头能回多少
      total_free = add(free_cash, long_cash)
      total_free ≤ 0 → position ≤ 0 ? Rejected·NoCashShort : 上限 = long_size
      否则 max_short = add(total_free, -fixed) / (adj_price × (1 + fees))
           limit = long_size + max_short；limit ≤ 0 → Rejected·CantCoverFees
      lock_cash 时在这里就把 percent 用掉（置 NaN）
  其他                              → size

percent 未用掉 → size_limit *= percent
size_limit > max_size → allow_partial ? = max_size : Rejected·MaxSizeExceeded
Short/Both: size_limit 无限 → Err（无限做空）
LongOnly:   position == 0 → Rejected·NoOpenPosition
粒度取整；is_close(size_limit,0) → Ignored·SizeZero；< min_size → MinSizeNotReached；部分成交检查

acq_cash  = size_limit × adj_price
fees_paid = acq_cash × fees + fixed
final_acq = add(acq_cash, -fees_paid)；< 0 → Rejected·CantCoverFees

new_cash     = cash + final_acq
new_position = add(position, -size_limit)
new_position < 0（有一部分变成空头）：
    short_size  = position < 0 ? size_limit : |new_position|            # 只有净空头部分产生负债
    short_value = short_size × adj_price
    new_debt      = debt + short_value
    new_free_cash = add(free_cash, add(final_acq, -2·short_value))
否则：new_free_cash = free_cash + final_acq
```

同一列的持仓在空仓、多头、空头之间移动。加多、减多、加空、减空都留在原来的状态里。买穿或卖穿是同一笔成交穿过 0，只按穿过的那一段改 `debt`。

```mermaid
flowchart LR
  flat["空仓"]
  lng["多头，debt = 0"]
  sht["空头<br/>debt = 数量 × 均价"]
  flat -- buy --> lng
  flat -- sell --> sht
  lng -- 卖光 --> flat
  lng -- 卖穿 --> sht
  sht -- 买光 --> flat
  sht -- 买穿 --> lng
  classDef base fill:#ddf4ff,stroke:#0969da;
  classDef hot fill:#ffebe9,stroke:#cf222e;
  class lng base;
  class sht hot;
```

**为什么是 `2 × short_value`**（`types.rs:9-17`）：开空收到 `short_value` 的现金，同时为"将来买回"预留等值现金，再为空头敞口占用一次。`free_cash` 的变化是 `proceeds − 2·short_value`。这段记账在 `new_position < 0` 的分支里总会发生，不看 `lock_cash`。`lock_cash` 只决定以后的订单是否遵守已经记下的预留（§2.6）。平空按 `debt / |position|` 对称释放。预留取决于每次开空的价格，只靠 `cash` 和 `position` 推不出来，所以 `debt` 和 `free_cash` 是显式状态。共享现金组里这两个现金字段是整组一份，`debt` 跟 `position` 一样每列一份（第 3 章）。
测试 `shorting_records_debt_and_reserves_margin`（`kernel.rs:883-897`）：100 现金、空 5 股 @10 → `cash=150, debt=50, free_cash=50`。

## 2.8 14 种 `StatusInfo` 的触发点

| `StatusInfo` | 状态 | 触发位置 |
|---|---|---|
| `SizeNaN` / `PriceNaN` | Ignored | `kernel.rs:585-596`，校验之前 |
| `ValueNaN` / `ValueZeroNeg` | Ignored / Rejected | TargetPercent 阶段 `kernel.rs:620-635` |
| `ValPriceNaN` | Ignored | Value/TargetValue 阶段 `kernel.rs:643-648` |
| `RandomEvent` | Rejected | 分发前 `kernel.rs:686-692` |
| `NoCashLong` | Rejected | buy，`cash_limit == 0` `kernel.rs:94-98` |
| `NoOpenPosition` | Rejected | buy（ShortOnly 无空头可平）`kernel.rs:105-110`；sell（LongOnly 无多头可卖）`kernel.rs:309-314` |
| `SizeZero` | Ignored | buy `kernel.rs:119-124, 172-177`；sell `kernel.rs:320-325` |
| `MaxSizeExceeded` | Rejected | buy `kernel.rs:126-134`；sell `kernel.rs:293-301` |
| `CantCoverFees` | Rejected | buy 反解 ≤ 0 `kernel.rs:151-156`；sell 两处 `kernel.rs:259-264, 342-347` |
| `MinSizeNotReached` | Rejected | buy `kernel.rs:178-183`；sell `kernel.rs:326-331` |
| `PartialFill` | Rejected | buy `kernel.rs:185-190`；sell `kernel.rs:332-337` |
| `NoCashShort` | Rejected | sell `kernel.rs:245-252` |

枚举定义和判别值在 `crates/barq-model/src/enums.rs`，记录里存的是整数，所以判别值被测试钉死。

## 2.9 性能结构：`ValidatedOrder`

位置：`kernel.rs:416-526`。14 项校验里有 7 项只看整列通常不变的字段（fees、slippage、min/max_size、granularity、reject_prob、cash_limit）。
`ValidatedOrder::new` 做一次，之后每根 K 线只 `set_bar` 四个会变的字段（size、price、size_type、direction）。
在稀疏工作负载上值 12–15%（`kernel.rs:456-462`），`benchmarks/README.md` 记录了 +30% 稀疏 / +22% 密集。
两个循环都用它（第 3 章）。

## 2.10 容差为什么在这里最要命

`kernel.rs:144`：`is_close_or_less(total_req_cash, cash_limit)` 而不是 `<=`。两者只在 `1e-9` 以内不同，
把它换成 `<=` 的变异一开始通过了全部测试（`ARCHITECTURE.zh-CN.md §7.5、§14.4`）。
解决办法有两层：用 `math.nextafter` 构造刚好在边界两侧的 fixture；再用 `size_granularity` 把 `1e-9` 的差异放大成一整手（5.0 对 4.0）。
相反，`buy`/`sell` 里对零的比较（`cash_limit == 0.0`、`adj_size == 0.0`、`position == 0.0`）是**精确**比较（`kernel.rs:43-44`），因为状态在入口已经被零钉过，再用 `is_close` 反而会改变哪些订单成交。

## 2.11 与 vectorbt 1.1.0 的对应

| barq | vectorbt (`vectorbt/portfolio/nb.py`) | 差异 |
|---|---|---|
| `execute_order` | `execute_order_nb(state, order)` | 多了 `reject_draw` 参数；随机拒单提前到分发前 |
| `buy` / `sell` | `buy_nb` / `sell_nb` | `cash_limit` 是新增字段，不进日志 |
| `validate_bar` + `param_violation` | `execute_order_nb` 开头的校验 | 拆成两半以便每列只做一次 |
| `ExecuteOrderState` / `ProcessOrderState` | `ExecuteOrderState` / `ProcessOrderState` | 同名同义 |
| `Order` 默认值 | `order_nb` 的默认参数 | 同 |
| `StatusInfo` 14 项 | `OrderStatusInfo` | 顺序和判别值一致 |

## 2.12 延伸资料

- vectorbt 文档 `Portfolio` 一节对 `size_type`、`direction`、`lock_cash`、`allow_partial` 的解释：<https://vectorbt.dev/api/portfolio/base/>；`nb` 模块：<https://vectorbt.dev/api/portfolio/nb/>。读 barq 的 `buy`/`sell` 前先把这两页的参数表看一遍。
- PEP 485（`math.isclose` 的相对 / 绝对容差定义）：<https://peps.python.org/pep-0485/>。
- 做空保证金的直观解释：Investopedia "Short Selling: Margin Requirements"，以及 Reg T 的 150% 规则；barq 的 `2 × short_value` 就是"收到的钱 + 等额保证金"的简化模型。
- 变异测试入门：Pitest 的 "Basic concepts" 页（概念与 Rust 无关）；Rust 工具 `cargo-mutants`。
- 更多见 [11-references.md](11-references.md) §B。

## 2.13 自测

1. `ShortOnly` 列里 `size = 5` 的订单走的是 `buy` 还是 `sell`？（提示：`kernel.rs:615-618` 翻一次符号，第 3 章 `prepare_order` 还翻一次。）
2. `percent = 0.5`、`cash = 100`、`cash_limit = 60` 时，买入最多花多少？（先 `min(100, 60)` 再乘 0.5 → 30。）
3. 为什么 `is_close(adj_size, 0.0)` 测的是粒度取整之后、反解之前的 `adj_size`？去 fixture 里找一行 `granularity` 的案例验证你的答案。
4. 平掉一半空头时 `free_cash` 变化多少？用 `avg_entry_price` 写出式子。
5. 持仓 −5 时一笔买 8 且买得起。`OrderRecords` 里有几条？`debt` 按多少股释放？若要看到「平掉的 5 股」和「剩下的 3 股」分成两段，该读 `buy` 还是第 4 章的 `exit_trades`？
