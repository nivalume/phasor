---
title: 加载 load 与面板 prepare
kind: algorithm
summary: load 逐批次读、解码、投影、校验，解码之后才按 available_time 过滤；prepare 以 available_time 的有序并集为行轴拼出列主序面板。
parent: phasor-data
related: [pit-alignment, python-boundary, column-axis, look-ahead]
tags: [data, arrow, loading]
sources:
  - crates/phasor-data/src/lib.rs:217-414
  - crates/phasor-py/src/data.rs:238-250
  - crates/phasor-py/python/phasor/data.py:58
  - examples/multi_data.py:28-36
order: 601
---

# 加载 load 与面板 prepare

[phasor-data](../crates/phasor-data.md) 的两步：`load` 把目录里的文件变成 `Dataset`（一串解码后的 Arrow 批次），`prepare` 把 K 线数据集拼成 `BarPanel`。两步的时间轴都是 `available_time`，原因见 [延迟与前视偏差](../concepts/look-ahead.md)。

## 位置

- `crates/phasor-data/src/lib.rs:217-332`（`load`）
- `crates/phasor-data/src/lib.rs:343-414`（`prepare`）
- `crates/phasor-py/src/data.rs:238-250`（Python 解码器按批次重新拿 GIL）
- `crates/phasor-py/python/phasor/data.py:58`（`order="F"` reshape）
- `examples/multi_data.py:28-36`（解码器把毫秒改成纳秒）

## `load`（`lib.rs:217-332`）

```text
schema.validate()                                   name 是单个路径分量；时间列是 UInt64；Columns 绑定的两列是 Utf8
items 非空，start < end
for item in items:
    key 唯一、paths 非空
    bar_type   = key.parse::<BarType>().ok()
    instrument = bar_type.instrument 或 key.parse::<InstrumentId>().ok()
    binding == Source 且 instrument 为 None → Err
    for path in paths:
        for raw in reader(path, input):             65,536 行一批
            raw   = project(raw, input)
            batch = project(decode(raw, &ctx)?, output)     ← 用户解码器在这里
            event_time / available_time 不得有 null
            Columns 绑定：每行拼出 InstrumentId 校验；键是 BarType 时每行标的必须等于它
            有 start/end：take_record_batch 保留 available_time ∈ [start, end)
            batches.push(DecodedBatch{ctx, batch})
```

过滤放在解码**之后**，因为解码器可能改时间单位（`multi_data.py:28-36` 把交易所的毫秒改成纳秒）。
批次不拼接，`Dataset.batches` 是列表，峰值内存被 65,536 行一批限制住。
解码器是 `FnMut`，Python 回调从 `py.detach` 的外层循环里按批次 `Python::attach` 重新拿 GIL（`phasor-py/src/data.rs:238-250`）。

## `prepare`（`lib.rs:343-414`）

```text
每个 key 必须能解析成 BarType；Global 绑定 → Err；output 必须有 Float64 的 open/high/low/close/volume
times: BTreeSet<u64> = 所有批次 available_time 的并集          有序、去重
time  = times 转成 Vec；n_rows = |time|；n_cols = |keys|
fields = 5 × vec![NaN; n_rows × n_cols]；present = vec![false; …]
for batch:
    col = keys.position(batch.key)                             列顺序 = 调用方给的键顺序
    for (r, t) in available_time:
        index = col × n_rows + time.binary_search(t)           列主序
        present[index] → Err("duplicate bar")
        五个字段逐个写入；null 保持 NaN
BarPanel { time, bar_types, values }
```

行轴是 **`available_time`**；缺失的 `(key, time)` 保持 NaN；同一 `(key, available_time)` 出现两次是错误，跨文件也算。复杂度 O(N log N)。Python 侧用 `order="F"` reshape 成 `(T, S)`（`data.py:58`）。

## 相关

- [phasor-data：数据层](../crates/phasor-data.md)：所属 crate；类图、调用链与阅读陷阱（`prepare` 要求每个键都是 `BarType`、`project` 从不转换类型）。
- [按时点对齐 align](../algorithms/pit-alignment.md)：`load` 得到的辅助数据集如何对齐到 `prepare` 得到的时间轴上。
- [Python 边界：F 序与一维的三种含义](../concepts/python-boundary.md)：为什么面板在 Python 侧必须是 `order="F"`。
- [列轴与列主序](../concepts/column-axis.md)：`index = col × n_rows + row` 的列主序布局。
- [延迟与前视偏差](../concepts/look-ahead.md)：行轴用 `available_time` 而不是 `event_time` 的理由。
