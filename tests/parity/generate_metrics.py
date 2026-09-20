#!/usr/bin/env python3
"""Generate differential fixtures for the return metrics, scalar and rolling.

Where ``generate_analysis.py`` runs a whole simulation and checks the metrics
at the end of it, this one feeds return series straight in. That is the only
way to reach the degenerate cases on purpose: a simulation will not readily
produce a column that is entirely missing, one that never moved, or one whose
benchmark never rose, and those are exactly where the metrics disagree.

The series are hand-built for the branch each one reaches:

* empty, one bar and two bars, around every ``len < 2`` guard;
* flat, so every volatility denominator is zero;
* monotone up, so there is no drawdown and no down-capture;
* gaps in several positions, and a series that is nothing but gaps;
* a benchmark that is flat, one that only rises, and one that only falls;
* infinities, which the percentile code special-cases in ways that read as
  arbitrary because they are.

Each is crossed with the parameters that change a branch rather than only a
number — an annualization factor of exactly 1 (which skips the omega
threshold conversion) and one at -1 (which has no real root), cutoffs at 0, 1
and in between, a nonzero risk-free rate.

The rolling forms are generated over the longer series at several window and
``min_periods`` combinations, including ``min_periods`` below the window so
the short leading windows are covered.

Usage::

    python tests/parity/generate_metrics.py

Setup is the same as the other generators; see the README in this directory.
"""

from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np
from vectorbt.returns import nb as ret_nb

COLUMNS = [
    "fn",
    "returns",
    "benchmark",
    "ann_factor",
    "risk_free",
    "required_return",
    "cutoff",
    "ddof",
    "window",
    "minp",
    "result",
    "note",
]

# Metrics reading only their own returns: name -> (kernel, extra parameter names).
SOLO = {
    "total_return": (lambda r, **k: ret_nb.cum_returns_final_1d_nb(r, 0.0), ()),
    "annualized_return": (
        lambda r, ann_factor, **k: ret_nb.annualized_return_1d_nb(r, ann_factor),
        ("ann_factor",),
    ),
    "annualized_volatility": (
        lambda r, ann_factor, **k: ret_nb.annualized_volatility_1d_nb(r, ann_factor),
        ("ann_factor",),
    ),
    "max_drawdown": (lambda r, **k: ret_nb.max_drawdown_1d_nb(r), ()),
    "calmar_ratio": (
        lambda r, ann_factor, **k: ret_nb.calmar_ratio_1d_nb(r, ann_factor),
        ("ann_factor",),
    ),
    "omega_ratio": (
        lambda r, ann_factor, risk_free, required_return, **k: ret_nb.omega_ratio_1d_nb(
            r, ann_factor, risk_free, required_return
        ),
        ("ann_factor", "risk_free", "required_return"),
    ),
    "sharpe_ratio": (
        lambda r, ann_factor, risk_free, **k: ret_nb.sharpe_ratio_1d_nb(
            r, ann_factor, risk_free
        ),
        ("ann_factor", "risk_free"),
    ),
    "downside_risk": (
        lambda r, ann_factor, required_return, **k: ret_nb.downside_risk_1d_nb(
            r, ann_factor, required_return
        ),
        ("ann_factor", "required_return"),
    ),
    "sortino_ratio": (
        lambda r, ann_factor, required_return, **k: ret_nb.sortino_ratio_1d_nb(
            r, ann_factor, required_return
        ),
        ("ann_factor", "required_return"),
    ),
    "tail_ratio": (lambda r, **k: ret_nb.tail_ratio_1d_nb(r), ()),
    "value_at_risk": (
        lambda r, cutoff, **k: ret_nb.value_at_risk_1d_nb(r, cutoff),
        ("cutoff",),
    ),
    "cond_value_at_risk": (
        lambda r, cutoff, **k: ret_nb.cond_value_at_risk_1d_nb(r, cutoff),
        ("cutoff",),
    ),
}

# Metrics reading a benchmark too.
PAIRED = {
    "information_ratio": (
        lambda r, b, ddof, **k: ret_nb.information_ratio_1d_nb(r, b, ddof),
        ("ddof",),
    ),
    "beta": (lambda r, b, **k: ret_nb.beta_1d_nb(r, b), ()),
    "alpha": (
        lambda r, b, ann_factor, risk_free, **k: ret_nb.alpha_1d_nb(
            r, b, ann_factor, risk_free
        ),
        ("ann_factor", "risk_free"),
    ),
    "capture": (
        lambda r, b, ann_factor, **k: ret_nb.capture_1d_nb(r, b, ann_factor),
        ("ann_factor",),
    ),
    "up_capture": (
        lambda r, b, ann_factor, **k: ret_nb.up_capture_1d_nb(r, b, ann_factor),
        ("ann_factor",),
    ),
    "down_capture": (
        lambda r, b, ann_factor, **k: ret_nb.down_capture_1d_nb(r, b, ann_factor),
        ("ann_factor",),
    ),
}


def fmt(v: float) -> str:
    v = float(v)
    if np.isnan(v):
        return "nan"
    if np.isinf(v):
        return "inf" if v > 0 else "-inf"
    return repr(v)


def floats(a) -> str:
    return ";".join(fmt(v) for v in np.asarray(a, dtype=float).ravel())


NAN = np.nan
INF = np.inf

SERIES: list[tuple[str, list[float]]] = [
    ("empty", []),
    ("one bar", [0.05]),
    ("one bar, missing", [NAN]),
    ("two bars", [0.05, -0.03]),
    ("flat at zero", [0.0, 0.0, 0.0, 0.0]),
    ("flat and nonzero", [0.02, 0.02, 0.02, 0.02]),
    ("monotone up", [0.01, 0.02, 0.03, 0.04, 0.05]),
    ("monotone down", [-0.01, -0.02, -0.03, -0.04, -0.05]),
    ("mixed", [0.01, -0.02, 0.03, -0.01, 0.02, 0.005, -0.015, 0.025]),
    ("a leading gap", [NAN, -0.02, 0.03, -0.01, 0.02]),
    ("a trailing gap", [0.01, -0.02, 0.03, -0.01, NAN]),
    ("a gap in the middle", [0.01, -0.02, NAN, -0.01, 0.02]),
    ("nothing but gaps", [NAN, NAN, NAN]),
    ("a total loss", [-1.0, 0.02, 0.03]),
    ("worse than a total loss", [-1.5, 0.02, 0.03]),
    ("a positive infinity", [0.01, INF, 0.03]),
    ("a negative infinity", [0.01, -INF, 0.03]),
    ("big swings", [0.5, -0.4, 0.8, -0.6, 0.3]),
]

BENCHMARKS: list[tuple[str, list[float]]] = [
    ("flat benchmark", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("benchmark only rises", [0.01, 0.02, 0.01, 0.03, 0.01, 0.02, 0.01, 0.02]),
    ("benchmark only falls", [-0.01, -0.02, -0.01, -0.03, -0.01, -0.02, -0.01, -0.02]),
    ("benchmark mixed", [0.005, -0.01, 0.02, -0.02, 0.01, 0.01, -0.005, 0.015]),
    ("benchmark with a gap", [0.005, NAN, 0.02, -0.02, 0.01, 0.01, -0.005, 0.015]),
]

ANN_FACTORS = [365.0, 252.0, 1.0, -1.0, -2.0]
CUTOFFS = [0.05, 0.0, 0.5, 1.0]


def blank() -> dict[str, str]:
    return {k: "" for k in COLUMNS}


def rows() -> list[dict[str, str]]:
    out: list[dict[str, str]] = []

    def add(fn, **fields):
        r = blank()
        r["fn"] = fn
        r.update({k: ("" if v is None else str(v)) for k, v in fields.items()})
        out.append(r)

    # --- scalar metrics over their own returns -----------------------------
    for note, vals in SERIES:
        r = np.array(vals, dtype=np.float64)
        rtext = floats(r)
        for name, (func, params) in SOLO.items():
            grids: list[dict[str, float]] = [{}]
            if "ann_factor" in params:
                grids = [{**g, "ann_factor": a} for g in grids for a in ANN_FACTORS]
            if "risk_free" in params:
                grids = [{**g, "risk_free": v} for g in grids for v in (0.0, 0.001)]
            if "required_return" in params:
                grids = [{**g, "required_return": v} for g in grids for v in (0.0, 0.01)]
            if "cutoff" in params:
                grids = [{**g, "cutoff": v} for g in grids for v in CUTOFFS]
            for grid in grids:
                full = {
                    "ann_factor": 365.0,
                    "risk_free": 0.0,
                    "required_return": 0.0,
                    "cutoff": 0.05,
                    **grid,
                }
                try:
                    value = func(r, **full)
                except Exception:
                    # The reference raises on some degenerate inputs; those
                    # are pinned by Rust unit tests rather than fixtures.
                    continue
                add(
                    name,
                    returns=rtext,
                    result=fmt(value),
                    note=note,
                    **{k: fmt(v) for k, v in grid.items()},
                )

    # --- scalar metrics against a benchmark --------------------------------
    eight = [
        ("mixed", [0.01, -0.02, 0.03, -0.01, 0.02, 0.005, -0.015, 0.025]),
        ("twice the benchmark", [0.01, -0.02, 0.04, -0.04, 0.02, 0.02, -0.01, 0.03]),
        ("flat at zero", [0.0] * 8),
        ("a gap in the middle", [0.01, -0.02, NAN, -0.01, 0.02, 0.005, -0.015, 0.025]),
        ("all gaps", [NAN] * 8),
    ]
    for rnote, rvals in eight:
        r = np.array(rvals, dtype=np.float64)
        for bnote, bvals in BENCHMARKS:
            b = np.array(bvals, dtype=np.float64)
            for name, (func, params) in PAIRED.items():
                grids: list[dict[str, float]] = [{}]
                if "ann_factor" in params:
                    grids = [{**g, "ann_factor": a} for g in grids for a in (365.0, 1.0)]
                if "risk_free" in params:
                    grids = [{**g, "risk_free": v} for g in grids for v in (0.0, 0.001)]
                if "ddof" in params:
                    grids = [{**g, "ddof": v} for g in grids for v in (0, 1)]
                for grid in grids:
                    full = {
                        "ann_factor": 365.0,
                        "risk_free": 0.0,
                        "ddof": 1,
                        **grid,
                    }
                    try:
                        value = func(r, b, **full)
                    except Exception:
                        continue
                    fields = {
                        k: (str(int(v)) if k == "ddof" else fmt(v))
                        for k, v in grid.items()
                    }
                    add(
                        name,
                        returns=floats(r),
                        benchmark=floats(b),
                        result=fmt(value),
                        note=f"{rnote} / {bnote}",
                        **fields,
                    )

    # --- rolling forms -----------------------------------------------------
    # `rolling_apply_nb` wants a 2-D array; one column is enough, since the
    # per-column loop is already covered elsewhere.
    long_series = [
        ("mixed", [0.01, -0.02, 0.03, -0.01, 0.02, 0.005, -0.015, 0.025, 0.01, -0.03]),
        ("with two gaps", [0.01, NAN, 0.03, -0.01, NAN, 0.005, -0.015, 0.025, 0.01, -0.03]),
        ("flat at zero", [0.0] * 10),
        ("monotone up", [0.01] * 10),
    ]
    long_bench = [0.005, -0.01, 0.02, -0.02, 0.01, 0.01, -0.005, 0.015, 0.0, -0.02]
    windows = [(3, None), (3, 1), (4, 2), (10, None), (1, None)]
    b2 = np.array(long_bench, dtype=np.float64).reshape(-1, 1)

    rolling_solo = {
        "total_return": lambda a, w, m: ret_nb.rolling_cum_returns_final_nb(a, w, m, 0.0),
        "annualized_return": lambda a, w, m: ret_nb.rolling_annualized_return_nb(a, w, m, 365.0),
        "annualized_volatility": lambda a, w, m: ret_nb.rolling_annualized_volatility_nb(
            a, w, m, 365.0
        ),
        "max_drawdown": lambda a, w, m: ret_nb.rolling_max_drawdown_nb(a, w, m),
        "calmar_ratio": lambda a, w, m: ret_nb.rolling_calmar_ratio_nb(a, w, m, 365.0),
        "omega_ratio": lambda a, w, m: ret_nb.rolling_omega_ratio_nb(a, w, m, 365.0, 0.0, 0.0),
        "sharpe_ratio": lambda a, w, m: ret_nb.rolling_sharpe_ratio_nb(a, w, m, 365.0, 0.0),
        "downside_risk": lambda a, w, m: ret_nb.rolling_downside_risk_nb(a, w, m, 365.0, 0.0),
        "sortino_ratio": lambda a, w, m: ret_nb.rolling_sortino_ratio_nb(a, w, m, 365.0, 0.0),
        "tail_ratio": lambda a, w, m: ret_nb.rolling_tail_ratio_nb(a, w, m),
        "value_at_risk": lambda a, w, m: ret_nb.rolling_value_at_risk_nb(a, w, m, 0.05),
        "cond_value_at_risk": lambda a, w, m: ret_nb.rolling_cond_value_at_risk_nb(a, w, m, 0.05),
    }
    rolling_paired = {
        "information_ratio": lambda a, w, m: ret_nb.rolling_information_ratio_nb(a, w, m, b2, 1),
        "beta": lambda a, w, m: ret_nb.rolling_beta_nb(a, w, m, b2),
        "alpha": lambda a, w, m: ret_nb.rolling_alpha_nb(a, w, m, b2, 365.0, 0.0),
        "capture": lambda a, w, m: ret_nb.rolling_capture_nb(a, w, m, b2, 365.0),
        "up_capture": lambda a, w, m: ret_nb.rolling_up_capture_nb(a, w, m, b2, 365.0),
        "down_capture": lambda a, w, m: ret_nb.rolling_down_capture_nb(a, w, m, b2, 365.0),
    }

    for note, vals in long_series:
        a2 = np.array(vals, dtype=np.float64).reshape(-1, 1)
        for window, minp in windows:
            for name, func in rolling_solo.items():
                try:
                    res = func(a2, window, minp)
                except Exception:
                    continue
                add(
                    f"rolling_{name}",
                    returns=floats(a2),
                    window=window,
                    minp="" if minp is None else minp,
                    result=floats(res),
                    note=note,
                )
            for name, func in rolling_paired.items():
                try:
                    res = func(a2, window, minp)
                except Exception:
                    continue
                add(
                    f"rolling_{name}",
                    returns=floats(a2),
                    benchmark=floats(b2),
                    window=window,
                    minp="" if minp is None else minp,
                    result=floats(res),
                    note=note,
                )

    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "fixtures" / "metrics.tsv",
    )
    args = parser.parse_args()

    data = rows()
    lines = ["\t".join(COLUMNS)]
    lines.extend("\t".join(r[k] for k in COLUMNS) for r in data)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines) + "\n")

    by: dict[str, int] = {}
    for r in data:
        by[r["fn"]] = by.get(r["fn"], 0) + 1
    print(f"wrote {len(data)} fixtures to {args.out}")
    for name in sorted(by):
        print(f"  {name}: {by[name]}")


if __name__ == "__main__":
    main()
