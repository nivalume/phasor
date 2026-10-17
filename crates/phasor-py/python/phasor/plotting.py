"""Charts of a portfolio, with Plotly. Needs ``pip install phasor[plot]``.

Each function returns a ``plotly.graph_objects.Figure`` to show, combine or
save; ``index`` puts the bars' times on the x axis::

    phasor.plotting.value(pf, 0, index=dates).show()
    phasor.plotting.orders(pf, 0, index=dates)
    phasor.plotting.drawdowns(pf, 0)
    phasor.plotting.rolling(pf, "sharpe_ratio", 63)
"""

from __future__ import annotations

from typing import Any

import numpy as np

__all__ = ["drawdowns", "orders", "rolling", "value"]

_UP, _DOWN, _LINE, _MUTED = "#2a9d8f", "#d1495b", "#3d5a80", "#98a6b3"


def _go():
    try:
        import plotly.graph_objects as go
    except ImportError as e:  # pragma: no cover - depends on the environment
        raise ImportError("phasor.plotting needs plotly: pip install phasor[plot]") from e
    return go


def _x(n: int, index: Any) -> np.ndarray:
    return np.arange(n) if index is None else np.asarray(index)


def _series(pf: Any, method: str, account: int) -> np.ndarray:
    f = getattr(pf, method)
    return f(group=account) if pf.cash_sharing else f(account)


def value(pf: Any, account: int = 0, *, index: Any = None, title: str | None = None) -> Any:
    """The account's value, and its drawdown from the running peak below
    it."""
    go = _go()
    from plotly.subplots import make_subplots

    v = _series(pf, "value", account)
    x = _x(len(v), index)
    peak = np.fmax.accumulate(v)
    with np.errstate(invalid="ignore", divide="ignore"):
        dd = v / peak - 1
    fig = make_subplots(rows=2, cols=1, shared_xaxes=True, row_heights=[0.7, 0.3], vertical_spacing=0.03)
    fig.add_trace(go.Scatter(x=x, y=v, name="value", line=dict(color=_LINE)), row=1, col=1)
    fig.add_trace(go.Scatter(x=x, y=dd, name="drawdown", fill="tozeroy", line=dict(color=_DOWN)),
                  row=2, col=1)
    fig.update_yaxes(tickformat=".0%", row=2, col=1)
    fig.update_layout(title=title or f"account {account}", hovermode="x unified")
    return fig


def orders(pf: Any, col: int = 0, *, index: Any = None, title: str | None = None) -> Any:
    """Column ``col``'s close with its fills: buys up, sells down."""
    go = _go()
    m = pf.column_map
    close = pf.close[:, col if m is None else m[col]]
    x = _x(len(close), index)
    o = pf.orders()
    mine = np.asarray(o["col"]) == col
    fig = go.Figure(go.Scatter(x=x, y=close, name="close", line=dict(color=_MUTED)))
    for side, name, color, symbol in ((0, "buy", _UP, "triangle-up"), (1, "sell", _DOWN, "triangle-down")):
        k = mine & (np.asarray(o["side"]) == side)
        fig.add_trace(go.Scatter(
            x=x[np.asarray(o["idx"])[k]], y=np.asarray(o["price"])[k], mode="markers", name=name,
            marker=dict(color=color, symbol=symbol, size=9),
            customdata=np.asarray(o["size"])[k], hovertemplate="%{y} x %{customdata}<extra>" + name + "</extra>",
        ))
    fig.update_layout(title=title or f"column {col}", hovermode="closest")
    return fig


def drawdowns(pf: Any, account: int = 0, *, index: Any = None, top: int = 5, title: str | None = None) -> Any:
    """The account's value with its ``top`` deepest drawdowns shaded."""
    go = _go()
    v = _series(pf, "value", account)
    x = _x(len(v), index)
    d = pf.drawdowns()
    mine = np.flatnonzero(np.asarray(d["col"]) == account)
    worst = mine[np.argsort(np.asarray(d["drawdown"])[mine])][:top]
    fig = go.Figure(go.Scatter(x=x, y=v, name="value", line=dict(color=_LINE)))
    for i in worst:
        a = int(d["peak_idx"][i])
        b = int(min(d["end_idx"][i], len(v) - 1))
        fig.add_vrect(x0=x[a], x1=x[b], fillcolor=_DOWN, opacity=0.12, line_width=0)
    fig.update_layout(title=title or f"account {account}: deepest drawdowns")
    return fig


def rolling(pf: Any, metric: str, window: int, *, index: Any = None, title: str | None = None, **kwargs: Any) -> Any:
    """``pf.rolling(metric, window)`` for every account."""
    go = _go()
    r = pf.rolling(metric, window, **kwargs)
    x = _x(r.shape[0], index)
    fig = go.Figure([go.Scatter(x=x, y=r[:, a], name=f"account {a}") for a in range(r.shape[1])])
    fig.update_layout(title=title or f"rolling {metric} ({window} bars)")
    return fig
