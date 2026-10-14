---
title: 按时点对齐 align
kind: algorithm
summary: 事件按标的分桶稳定排序，再用两个单调前进的指针做 as-of 与左开右闭窗口聚合，每个标的 O(T + E)，每格 O(1)。
parent: phasor-data
related: [data-loading, look-ahead, monotonic-queue, rolling-accumulators]
tags: [data, as-of, two-pointer, point-in-time]
sources:
  - crates/phasor-data/src/lib.rs:456-552
  - crates/phasor-py/tests/test_data.py:100-110
order: 602
---

# 按时点对齐 align

`align` 把一个辅助数据集（资金费率、新闻、基本面）的某个字段对齐到 [phasor-data](../crates/phasor-data.md) 面板的时间轴上，得到 `T × S` 的矩阵：要么取"此刻最后一个已发布的值"（as-of），要么对最近一段时间内发布的事件求和、计数或取均值。用的是发布时间 `available_time`，所以结果在每个时刻都只包含当时已经能看到的信息（见 [延迟与前视偏差](../concepts/look-ahead.md)）。

## 位置

- `crates/phasor-data/src/lib.rs:456-484`（`events`：按标的分桶并稳定排序）
- `crates/phasor-data/src/lib.rs:487-552`（`align`：双指针 as-of / 窗口聚合）
- `crates/phasor-py/tests/test_data.py:100-110`（并列事件与窗口边界的例子）

## 分桶与稳定排序：`events`

先 `events(field)`（`lib.rs:456-484`）：把该字段转成 `Float64`，按 `instrument_at(batch, row)` 分桶成 `BTreeMap<Option<InstrumentId>, Vec<(available_time, value)>>`（`None` 键是全局事件），每桶 `sort_by_key(时间)`——Rust 的 `sort_by_key` 是**稳定**排序，所以同一时刻的事件按"源 → 文件（已排序）→ 行"的顺序保留。

## 双指针 as-of 与窗口聚合

然后对每个输出列做一次**双指针扫描**，不是二分查找：

```text
rows = events[Global ? None : Some(instrument)]
left = right = 0; sum = 0; count = 0
for (i, t) in time:                                      time 严格递增
    while right < rows.len() && rows[right].t <= t:      进入窗口：available_time ≤ t（闭）
        if finite(v): sum += v; count += 1
        right += 1
    if 有窗口 (width, agg):
        if boundary = t.checked_sub(width):              u64 下溢 → 永不驱逐
            while left < right && rows[left].t <= boundary:   离开窗口：≤ t − width
                if finite(v): sum −= v; count −= 1
                left += 1
        if count == 0: sum = 0.0                          防漂移
        value = Count → right − left（含 NaN 行）
                count == 0 → empty
                Sum → sum；Mean → sum / count
    else:
        value = right == 0 ? NaN : rows[right−1].v        as-of：最后一个 available_time ≤ t 的值
    output[col × T + i] = value
```

窗口是**左开右闭** `(t − width, t]`：刚好 `width` 之前的事件被驱逐，刚好在 `t` 的事件被纳入。
`test_data.py:100-110` 的例子：事件 `available=[10,20,20,35]`、值 `[1,2,4,8]`、面板 `[0,10,20,30,40]`、宽 20 → as-of `[NaN,1,4,4,8]`（t=20 时并列事件取最后一个），sum `[0,1,7,6,8]`，count `[0,1,3,2,1]`。

## 为什么是双指针

为什么双指针：`time` 和 `rows` 都有序，两个指针单调前进，每个标的 O(T + E) 而不是 O(T log E)，运行和与计数让每个输出格 O(1)。全局事件对每一列重走同一个 `rows`，这就是"广播"。
数值上没有 Kahan 补偿，靠 `count == 0 → sum = 0` 在窗口清空时归零。

## 相关

- [phasor-data：数据层](../crates/phasor-data.md)：所属 crate；`Count` 含 NaN 行、`events()` 每次重新排序等阅读陷阱。
- [加载 load 与面板 prepare](../algorithms/data-loading.md)：`align` 的输入（`load` 出的数据集）和时间轴（`prepare` 出的面板）从哪里来。
- [延迟与前视偏差](../concepts/look-ahead.md)：`available_time` 是数据层挡住前视的那道机制。
- [单调队列：窗口极值与极值位置](../algorithms/monotonic-queue.md)：另一个靠"下标单调前进、每个元素进出各一次"得到均摊 O(1) 的窗口算法。
- [O(1) 滚动累加器](../algorithms/rolling-accumulators.md)：因子引擎的运行和如何用周期重建控制漂移，对照这里只在窗口清空时归零。
