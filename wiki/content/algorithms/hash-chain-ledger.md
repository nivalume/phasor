---
title: 哈希链账本
kind: algorithm
summary: 每条账本记录带上一条的哈希并逐条 git 提交，打开时逐行校验；pipeline_version 让改判断代码使旧的通过变 stale。
parent: research-protocol
related: [research-gates, research-harness, parity-testing]
tags: [research, ledger, hashing]
sources:
  - research/barq_lab/protocol/ledger.py:25-89
  - research/barq_lab/protocol/versions.py
  - research/tests/test_ledger.py
order: 101
---

# 哈希链账本

[研究协议](../tooling/research-protocol.md)的每一步（提出、批准、样本内运行、OOS、forward）都追加一条记录到 `ledger.jsonl`。记录之间用哈希串成链，所以事后改一行或删一行都会在下次打开时被发现；账本里的计数（样本内测试数 N、OOS 批次数）又直接决定[门槛](../algorithms/research-gates.md)有多高。

## 位置

- `research/barq_lab/protocol/ledger.py:32-34`（`_digest`）
- `research/barq_lab/protocol/ledger.py:58-66`（`_read` 校验）
- `research/barq_lab/protocol/ledger.py:68-89`（`append`、`commit`）
- `research/barq_lab/protocol/versions.py`（`pipeline_version`）
- `research/tests/test_ledger.py`（篡改测试）

## 追加与校验

```text
entry = { seq, time(UTC ISO 秒), kind, prev_hash = 上一条的 hash 或 GENESIS("0"×64), …payload }
entry.hash = sha256( json.dumps(除 hash 外的全部字段, sort_keys=True, ensure_ascii=False) )
追加到 ledger.jsonl → git add → git -c user.name=barq-lab commit（连同 reports/）
打开时 _read 逐行校验 prev_hash == 上一条.hash 且 hash == digest(entry)，否则 LedgerError("ledger chain broken at line N")
```

## 篡改测试与 pipeline_version

`test_ledger.py` 做篡改测试（改一行、删一行）。`pipeline_version = sha256(policy_version + barq_lab 下除 llm/pdf/cli/propose 外所有 .py 的路径与字节)`：改判断代码让旧的通过变"stale"，改提示词不影响。

## 相关

- [研究协议 barq_lab](../tooling/research-protocol.md)：账本记录的就是状态机上的每一次转移。
- [门槛、多重比较与原创性检查](../algorithms/research-gates.md)：Bonferroni 的 N 和 OOS 的 M 从账本里数出来。
- [评估框架与合同检查](../algorithms/research-harness.md)：被 `pipeline_version` 覆盖的判断代码主要在这里。
- [差分测试与 fixture](../tooling/parity-testing.md)：另一种"结果一旦提交就不许悄悄改"的机制——fixture 不许为了通过而重新生成。
