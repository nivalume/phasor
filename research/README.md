# phasor-lab

`phasor-lab` is a pre-registered research protocol for phasor factor experiments. The agent writes hypotheses and implementations; a fixed protocol decides the outcome. The LLM never sees a result, so it cannot optimise against the data.

```
propose ─► review/approve ─► run (once, in-sample) ─► gate ─► oos (batched) ─► forward (shadow) ─► report
  LLM         human            fixed harness        fixed      budgeted          new bars only
```

## Commands

The ledger repository is `PHASOR_LAB_LEDGER`, which defaults to `../phasor-ledger` next to this repo. Commands must run in an environment with phasor installed, such as `rdagent4phasor`.

```bash
pip install -e research
phasor-lab propose --symbols BTCUSDT --prompt "trend following: established moves persist" --n 6
phasor-lab propose --symbols BTCUSDT --pdf paper.pdf --pages 38-52 --n 6
phasor-lab propose --symbols BTCUSDT --n 6            # no direction: the agent covers factor families
phasor-lab review                                     # edit pending/<id>.yaml by hand if needed
phasor-lab approve <id> [<id> ...]                    # freezes card+spec; `repair <id>` if its check fails
phasor-lab run                                        # every registered spec once, 2021-01..2025-01
phasor-lab oos                                        # batch of current in-sample passes (>= 90 days apart, 3 batches max)
phasor-lab forward                                    # survivors on bars newer than their registration
phasor-lab report
phasor-lab calibrate --symbols BTCUSDT                # negative and positive controls
phasor-lab import-legacy                              # trials made on the same data before the ledger
```

The LLM is configured with `PHASOR_LAB_MODEL`, `PHASOR_LAB_API_BASE` and `PHASOR_LAB_API_KEY`. If they are not set, it uses `CHAT_MODEL` and `OPENAI_*` from `../RD-Agent/.env`.

## What the protocol enforces

- **Cards before data.** A card records the mechanism, why the effect exists, the counterparty, the horizon, the failure conditions, a fixed direction, and parameter ranges justified by theory. Cards may not mention years, dates, months or named events, because the LLM's training data covers the evaluation periods.
- **One test per spec.** The parameter grid is an equal-weight ensemble, or a walk-forward selection whose rule is declared in advance. Specs with the same structure as anything already tested are rejected, and swapping one price field for another does not count as a new structure.
- **Thresholds that rise with the ledger.** The in-sample gate requires t of daily net returns ≥ Φ⁻¹(1 − α/2N), where N is the global number of in-sample tests. The gate also requires:
  - Sharpe above 0 at 2× costs;
  - at least 60% of grid members positive;
  - at least 75% of the BTC/ETH/SOL/BNB universe positive, as a robustness check rather than extra sample;
  - t ≤ 8, since anything higher is treated as a look-ahead bug.
  Every other number in a report is labelled *reference only*.
- **The ledger cannot be reset.** `ledger.jsonl` is hash-chained and git-committed on every append. `policy.yaml` and the judging code are versioned, and changing either marks earlier passes stale.
- **Out of sample is batched and budgeted.** 2025 is out of sample for the pipeline but not for the LLM. Clean evidence therefore comes only from forward shadow tracking, with an interim verdict at 90 days and a final one at 180. A factor is retired if its forward returns deviate from the in-sample distribution (z < −2).

A healthy pipeline mostly rejects. Funding payments are not modelled yet, and every report says so.

## Adding an experiment type

A new type implements `protocol/experiment_type.py`: spec schema, regulation, contract check, run, gates and report sections. Add one only when a card needs it, for example path-dependent rule strategies, event studies or carry.
