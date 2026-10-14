"""Generate the architecture diagrams in docs/img/{en,zh}/*.svg.

Run from the repository root:

    python docs/img/generate.py

Each diagram is defined once and rendered twice, with English and Chinese
labels. Identifiers (crate, type and function names) are the same in both
languages. The SVGs carry their own light background so they stay legible when
GitHub shows the page in dark mode.
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from xml.sax.saxutils import escape

ROOT = os.path.dirname(os.path.abspath(__file__))

FG = "#1f2328"
MUTED = "#59636e"
LINE = "#8c959f"
BORDER = "#d1d9e0"
BOX = "#f6f8fa"
ACCENT, ACCENT_BG = "#0969da", "#ddf4ff"
WARN, WARN_BG = "#cf222e", "#ffebe9"
OK, OK_BG = "#1a7f37", "#dafbe1"
PURPLE, PURPLE_BG = "#8250df", "#fbefff"
AMBER, AMBER_BG = "#9a6700", "#fff8c5"
FONT = (
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Noto Sans', Helvetica, Arial, "
    "'PingFang SC', 'Hiragino Sans GB', 'Microsoft YaHei', 'Noto Sans CJK SC', sans-serif"
)
MONO = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"

TONES = {
    "plain": (FG, BOX, BORDER),
    "accent": (ACCENT, ACCENT_BG, ACCENT),
    "warn": (WARN, WARN_BG, WARN),
    "ok": (OK, OK_BG, OK),
    "purple": (PURPLE, PURPLE_BG, PURPLE),
    "amber": (AMBER, AMBER_BG, AMBER),
    "ghost": (MUTED, "#ffffff", BORDER),
}


def text_width(s: str, size: float, mono: bool = False) -> float:
    """Rough rendered width: CJK glyphs are square; Latin ~0.52 em, monospace 0.6 em."""
    latin = 0.6 if mono else 0.52
    return sum(size if ord(c) > 0x2E80 else size * latin for c in s)


@dataclass
class Svg:
    width: int
    height: int
    title: str
    parts: list[str] = field(default_factory=list)
    problems: list[str] = field(default_factory=list)

    def add(self, s: str) -> None:
        self.parts.append(s)

    def text(self, x, y, s, size=13, anchor="middle", color=FG, weight="normal", mono=False):
        family = MONO if mono else FONT
        w = text_width(s, size, mono)
        left = {"start": x, "middle": x - w / 2, "end": x - w}[anchor]
        if left < 4 or left + w > self.width - 4:
            self.problems.append(f"text outside the canvas: {s!r}")
        self.add(
            f'<text x="{x:.1f}" y="{y:.1f}" font-family="{family}" font-size="{size}" '
            f'fill="{color}" text-anchor="{anchor}" font-weight="{weight}" '
            f'dominant-baseline="middle">{escape(s)}</text>'
        )

    def box(self, x, y, w, h, title, sub=None, tone="plain", mono_sub=True, size=13, dash=False):
        color, fill, stroke = TONES[tone]
        for label, px, mono in ((title, size, False), (sub or "", 11, mono_sub)):
            if text_width(label, px, mono) > w - 8:
                self.problems.append(f"label wider than its box: {label!r}")
        dashes = ' stroke-dasharray="5 4"' if dash else ""
        self.add(
            f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="6" '
            f'fill="{fill}" stroke="{stroke}" stroke-width="1.2"{dashes}/>'
        )
        if sub:
            self.text(x + w / 2, y + h / 2 - 8, title, size=size, color=color, weight="600")
            self.text(x + w / 2, y + h / 2 + 10, sub, size=11, color=MUTED, mono=mono_sub)
        else:
            self.text(x + w / 2, y + h / 2, title, size=size, color=color, weight="600")

    def arrow(self, x1, y1, x2, y2, label=None, color=LINE, dash=False, label_side="right",
              label_dx=0, label_dy=0, head=True):
        dashes = ' stroke-dasharray="5 4"' if dash else ""
        marker = ' marker-end="url(#arrow)"' if head else ""
        self.add(
            f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" stroke="{color}" '
            f'stroke-width="1.4"{dashes}{marker}/>'
        )
        if label:
            mx, my = (x1 + x2) / 2 + label_dx, (y1 + y2) / 2 + label_dy
            if x1 == x2:  # vertical: label beside the line
                anchor = "start" if label_side == "right" else "end"
                self.text(mx + (8 if label_side == "right" else -8), my, label, size=11,
                          color=MUTED, anchor=anchor)
            else:
                self.text(mx, my - 9, label, size=11, color=MUTED)

    def path(self, d, color=LINE, dash=False, head=True):
        dashes = ' stroke-dasharray="5 4"' if dash else ""
        marker = ' marker-end="url(#arrow)"' if head else ""
        self.add(f'<path d="{d}" fill="none" stroke="{color}" stroke-width="1.4"{dashes}{marker}/>')

    def rect(self, x, y, w, h, fill, stroke=BORDER, rx=0, dash=False):
        dashes = ' stroke-dasharray="4 3"' if dash else ""
        self.add(
            f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{rx}" '
            f'fill="{fill}" stroke="{stroke}" stroke-width="1"{dashes}/>'
        )

    def brace(self, x1, x2, y, label, color=MUTED, up=False):
        d = -6 if up else 6
        self.add(
            f'<path d="M{x1:.1f},{y:.1f} v{d} H{x2:.1f} v{-d}" fill="none" stroke="{color}" '
            f'stroke-width="1.2"/>'
        )
        self.text((x1 + x2) / 2, y + (d * 2.6), label, size=11, color=color)

    def render(self) -> str:
        head = (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.width} {self.height}" '
            f'width="{self.width}" height="{self.height}" role="img" aria-label="{escape(self.title)}">'
            f"<title>{escape(self.title)}</title>"
            '<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" '
            f'markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="{LINE}"/>'
            "</marker></defs>"
            f'<rect x="0.5" y="0.5" width="{self.width - 1}" height="{self.height - 1}" rx="10" '
            f'fill="#ffffff" stroke="{BORDER}"/>'
        )
        return head + "".join(self.parts) + "</svg>\n"


# ---------------------------------------------------------------- diagrams --


def pipeline(t):
    s = Svg(1000, 610, t("pipeline_title"))
    x, w = 290, 420
    stages = [
        ("files", "phasor-data", "plain"),
        ("panel", "BarPanel · OhlcvPanel", "plain"),
        ("factors", "phasor-expr · FactorSet", "accent"),
        ("signals", "SignalSweep · @phasor.strategy", "plain"),
        ("sim", "phasor-sim → phasor-exec", "accent"),
        ("records", "phasor-records", "plain"),
        ("analysis", "phasor-analysis · Portfolio", "plain"),
    ]
    shapes = [
        "CSV · Parquet · catalog",
        "T × S",
        "T × S  " + t("per_factor"),
        "T × (P × S)",
        "",
        "orders · logs → trades · drawdowns",
        "value · returns · metrics · stats",
    ]
    h, gap, y0 = 52, 32, 40
    for i, ((key, sub, tone), shape) in enumerate(zip(stages, shapes)):
        y = y0 + i * (h + gap)
        s.box(x, y, w, h, t(key), sub, tone=tone)
        if shape:
            s.text(x + w + 18, y + h / 2, shape, size=12, color=MUTED, anchor="start", mono=True)
        if i:
            s.arrow(x + w / 2, y - gap, x + w / 2, y, t(f"edge_{i}"), label_side="left",
                    label_dx=-w / 2 + 10)
    # side notes
    s.text(x - 20, y0 + 4 * (h + gap) + h / 2, t("parallel_accounts"), size=12, color=ACCENT,
           anchor="end")
    s.text(x - 20, y0 + 2 * (h + gap) + h / 2, t("parallel_symbols"), size=12, color=ACCENT,
           anchor="end")
    return s


def crates(t):
    s = Svg(1000, 470, t("crates_title"))
    W, H = 150, 46
    pos = {
        "phasor-core": (425, 400),
        "phasor-model": (220, 310),
        "phasor-generic": (520, 310),
        "phasor-signals": (760, 310),
        "phasor-exec": (40, 220),
        "phasor-data": (220, 220),
        "phasor-indicators": (620, 220),
        "phasor-expr": (430, 220),
        "phasor-records": (40, 130),
        "phasor-sim": (220, 130),
        "phasor-analysis": (220, 40),
        "phasor": (560, 40),
        "phasor-py": (800, 130),
    }
    tone = {"phasor-expr": "accent", "phasor-sim": "accent", "phasor-exec": "accent", "phasor-py": "purple",
            "phasor": "purple", "phasor-data": "amber"}
    for name, (x, y) in pos.items():
        s.box(x, y, W, H, name, tone=tone.get(name, "plain"), size=12)

    def edge(a, b):
        ax, ay = pos[a]
        bx, by = pos[b]
        if ay == by:  # same row: side to side
            s.arrow(ax, ay + H / 2, bx + W, by + H / 2, color=LINE)
            return
        # from bottom of a to top of b (dependency arrow points at the dependency)
        s.arrow(ax + W / 2, ay + H, bx + W / 2, by, color=LINE)

    for a, b in [
        ("phasor-model", "phasor-core"), ("phasor-generic", "phasor-core"), ("phasor-signals", "phasor-core"),
        ("phasor-exec", "phasor-model"), ("phasor-data", "phasor-model"), ("phasor-expr", "phasor-generic"),
        ("phasor-expr", "phasor-model"), ("phasor-indicators", "phasor-generic"),
        ("phasor-records", "phasor-exec"), ("phasor-sim", "phasor-records"), ("phasor-sim", "phasor-exec"),
        ("phasor-analysis", "phasor-sim"),
    ]:
        edge(a, b)
    s.text(40, 25, t("crates_note"), size=11, color=MUTED, anchor="start")
    s.text(800 + W / 2, 196, "→ data, expr, sim, analysis …", size=10, color=MUTED, mono=True)
    s.text(560 + W / 2, 106, t("facade"), size=10, color=MUTED)
    return s


def columns(t):
    s = Svg(1000, 450, t("columns_title"))
    x0, y0, cw, rh = 200, 96, 90, 30
    params = ["10", "10", "20", "20", "50", "50"]
    syms = ["BTC", "ETH"] * 3
    s.text(x0 - 12, y0 - 34, "col", size=12, color=MUTED, anchor="end", mono=True)
    s.text(x0 - 12, y0 - 12, "window", size=12, color=MUTED, anchor="end", mono=True)
    s.text(x0 - 12, y0 + 10, "symbol", size=12, color=MUTED, anchor="end", mono=True)
    tones = [ACCENT_BG, ACCENT_BG, OK_BG, OK_BG, AMBER_BG, AMBER_BG]
    for c in range(6):
        cx = x0 + c * cw
        s.text(cx + cw / 2, y0 - 34, str(c), size=12, mono=True)
        s.rect(cx, y0 - 24, cw, 24, tones[c])
        s.text(cx + cw / 2, y0 - 12, params[c], size=12, mono=True)
        s.rect(cx, y0, cw, 22, "#ffffff")
        s.text(cx + cw / 2, y0 + 11, syms[c], size=12, mono=True)
        for r in range(4):
            s.rect(cx, y0 + 22 + r * rh, cw, rh, "#ffffff")
            s.text(cx + cw / 2, y0 + 22 + r * rh + rh / 2, f"t{r}", size=11, color=MUTED, mono=True)
    s.text(x0 - 12, y0 + 22 + 2 * rh, t("time"), size=12, color=MUTED, anchor="end")
    for g in range(3):
        s.brace(x0 + g * 2 * cw + 4, x0 + (g + 1) * 2 * cw - 4, y0 + 22 + 4 * rh + 8,
                t("account") + f" {g}", color=ACCENT)
    s.text(x0 + 3 * cw, 26, "col = param_idx × n_symbols + symbol_idx", size=13, mono=True,
           weight="600")
    # memory strip
    y = 350
    s.text(40, y - 26, t("memory"), size=12, color=MUTED, anchor="start")
    cell = 28
    for c in range(6):
        for r in range(4):
            i = c * 4 + r
            s.rect(40 + i * (cell + 2) + c * 6, y, cell, 26, tones[c])
            s.text(40 + i * (cell + 2) + c * 6 + cell / 2, y + 13, f"{c}.{r}", size=9, mono=True)
    s.text(40, y + 50, "data[col × n_rows + row]  ·  " + t("contiguous"), size=12, anchor="start",
           mono=False, color=FG)
    return s


def graph(t):
    s = Svg(1040, 490, t("graph_title"))
    # left: three expressions
    s.text(40, 40, t("two_factors"), size=12, color=MUTED, anchor="start")
    s.box(40, 58, 280, 40, "ROC5", "Ref($close, 5) / $close", tone="ghost")
    s.box(40, 108, 280, 40, "RET", "$close / Ref($close, 1) - 1", tone="ghost")
    s.box(40, 158, 280, 40, "KMID", "($close - $open) / $open", tone="ghost")
    s.arrow(330, 128, 380, 128, t("compile"))
    # graph nodes: fields at the bottom, outputs at the top
    N = {
        "close": (430, 400, "$close", "field", "plain"),
        "open": (690, 400, "$open", "field", "plain"),
        "ref5": (400, 290, "Ref(·,5)", t("view"), "ghost"),
        "ref1": (540, 290, "Ref(·,1)", t("view"), "ghost"),
        "sub": (680, 290, "−", "slot 0", "accent"),
        "div1": (400, 200, "÷", "→ ROC5", "ok"),
        "div2": (540, 200, "÷", "slot 0", "accent"),
        "minus": (540, 110, "− 1", "→ RET", "ok"),
        "div3": (680, 200, "÷", "→ KMID", "ok"),
    }
    W, H = 110, 44
    for k, (x, y, a, b, tone) in N.items():
        s.box(x, y, W, H, a, b, tone=tone, size=12, mono_sub=(tone != "ghost"))

    def up(a, b):
        ax, ay = N[a][:2]
        bx, by = N[b][:2]
        s.arrow(bx + W / 2, ay, bx + W / 2, by + H)

    for a, b in [("close", "ref5"), ("open", "sub"), ("ref5", "div1"), ("ref1", "div2"),
                 ("div2", "minus"), ("sub", "div3")]:
        up(a, b)
    # the remaining edges are routed through the gaps between nodes
    mid = 200 + H / 2
    s.path(f"M445,400 V376 H385 V{mid} H400")          # $close -> ROC5 ÷
    s.path(f"M525,400 V{mid} H540")                    # $close -> RET ÷
    s.path("M540,412 H570 V334")                       # $close -> Ref(·,1)
    s.path("M540,432 H615 V356 H710 V334")             # $close -> −
    s.path(f"M790,420 H815 V{mid} H790")               # $open -> KMID ÷
    # right: notes
    notes = ["hash_consing", "validation", "folding", "order", "slots"]
    for i, k in enumerate(notes):
        s.text(830, 120 + i * 50, f"{i + 1}. " + t(k), size=12, anchor="start")
        s.text(846, 140 + i * 50, t(k + "_sub"), size=11, color=MUTED, anchor="start")
    s.text(560, 470, t("graph_caption"), size=11, color=MUTED)
    return s


def evaluate(t):
    s = Svg(1000, 430, t("eval_title"))
    colors = [(ACCENT, ACCENT_BG), (OK, OK_BG), (PURPLE, PURPLE_BG)]
    assign = [0, 1, 2, 0, 1, 2]
    cols = 6
    # inputs: one column per symbol, coloured by the thread that picks it up
    s.text(40, 36, t("inputs"), size=12, color=MUTED, anchor="start")
    for c in range(cols):
        col, bg = colors[assign[c]]
        s.rect(40 + c * 40, 50, 34, 300, bg, stroke=col)
        s.text(57 + c * 40, 200, f"S{c}", size=11, mono=True, color=col)
    # threads
    for th in range(3):
        y = 50 + th * 110
        c, bg = colors[th]
        s.arrow(285, y + 45, 325, y + 45, color=c)
        s.rect(330, y, 300, 90, bg, stroke=c, rx=6)
        s.text(480, y + 16, t("thread") + f" {th}", size=12, color=c, weight="600")
        s.text(480, y + 38, t("scratch"), size=11, color=MUTED)
        for k in range(6):
            s.rect(360 + k * 40, y + 52, 34, 22, "#ffffff", stroke=c)
            s.text(377 + k * 40, y + 63, f"{k}", size=10, mono=True, color=c)
        s.arrow(630, y + 45, 695, y + 45, color=c)
    # outputs
    s.text(700, 36, t("outputs"), size=12, color=MUTED, anchor="start")
    for f in range(3):
        y = 50 + f * 110
        s.text(700, y + 8, ["ROC5", "RET", "KMID"][f], size=11, anchor="start", mono=True)
        for c in range(cols):
            col, bg = colors[assign[c]]
            s.rect(700 + c * 44, y + 18, 40, 64, bg, stroke=col)
    s.text(500, 400, t("eval_caption"), size=11, color=MUTED)
    return s


def sliding(t):
    s = Svg(1000, 330, t("window_title"))
    x0, y0, cw = 60, 90, 44
    vals = ["3.1", "2.9", "NaN", "3.4", "3.0", "3.2", "3.3", "2.8", "3.5", "3.1"]
    for i, v in enumerate(vals):
        fill = BOX
        if 4 <= i <= 7:
            fill = ACCENT_BG
        if i == 8:
            fill = OK_BG
        if i == 3:
            fill = WARN_BG
        s.rect(x0 + i * cw, y0, cw - 4, 36, fill, stroke=BORDER, rx=4)
        s.text(x0 + i * cw + (cw - 4) / 2, y0 + 18, v, size=11, mono=True,
               color=MUTED if v == "NaN" else FG)
        s.text(x0 + i * cw + (cw - 4) / 2, y0 - 12, f"t{i}", size=10, mono=True, color=MUTED)
    s.brace(x0 + 4 * cw, x0 + 9 * cw - 4, y0 + 44, t("window_n"), color=ACCENT)
    # pop / push
    s.path(f"M{x0 + 3 * cw + 20},{y0 + 40} C{x0 + 3 * cw + 20},{y0 + 110} {x0 + 3 * cw},{y0 + 130} "
           f"{x0 + 4 * cw},{y0 + 140}", color=WARN)
    s.text(x0 + 3 * cw + 6, y0 + 110, t("pop"), size=11, color=WARN, anchor="end")
    s.path(f"M{x0 + 8 * cw + 20},{y0 + 40} C{x0 + 8 * cw + 20},{y0 + 110} {x0 + 7 * cw},{y0 + 130} "
           f"{x0 + 6 * cw},{y0 + 140}", color=OK)
    s.text(x0 + 8 * cw + 26, y0 + 120, t("push"), size=11, color=OK, anchor="start")
    s.box(x0 + 2 * cw, y0 + 140, 7 * cw - 4, 50, t("accumulator"),
          "count · Σ(x−shift) · Σ(x−shift)²", tone="accent", size=12)
    # right side notes
    notes = ["rebuild", "shift", "constant", "nan"]
    for i, k in enumerate(notes):
        s.text(560, 100 + i * 48, "• " + t(k), size=12, anchor="start")
        s.text(572, 120 + i * 48, t(k + "_sub"), size=11, color=MUTED, anchor="start")
    s.text(500, 36, t("window_head"), size=13, weight="600")
    return s


def cascade(t):
    s = Svg(900, 770, t("cascade_title"))
    steps = [
        ("ShortOnly", "size = −size", None),
        ("TargetPercent", "× value → TargetValue", "ValueNaN / ValueZeroNeg"),
        ("Value · TargetValue", "÷ val_price → Amount · TargetAmount", "ValPriceNaN"),
        ("TargetAmount", "− position → Amount", None),
        ("−inf (Short/Both)", "→ Percent(−1)", None),
        ("Percent", "percent = |size|, size = ±inf", None),
    ]
    x, w, h, gap, y0 = 230, 330, 46, 22, 70
    s.box(x, 20, w, 36, "Order { size, size_type, direction }", tone="ghost", size=12)
    for i, (a, b, fail) in enumerate(steps):
        y = y0 + i * (h + gap)
        s.box(x, y, w, h, f"{i + 1}. {a}", b, tone="plain", size=12)
        s.arrow(x + w / 2, y - gap + (0 if i else 8), x + w / 2, y)
        if fail:
            s.box(x + w + 60, y + 4, 230, h - 8, fail, tone="warn", size=11)
            s.arrow(x + w, y + h / 2, x + w + 60, y + h / 2, t("ignored_rejected"))
    yb = y0 + len(steps) * (h + gap)
    s.arrow(x + w / 2, yb - gap, x + w / 2, yb)
    s.box(x, yb, w, 40, t("dispatch"), tone="accent", size=12)
    s.box(60, yb + 80, 320, 56, "buy()", t("buy_sub"), tone="ok", size=12, mono_sub=False)
    s.box(410, yb + 80, 320, 56, "sell()", t("sell_sub"), tone="ok", size=12, mono_sub=False)
    s.arrow(x + 90, yb + 40, 220, yb + 80, "size > 0")
    s.arrow(x + w - 90, yb + 40, 570, yb + 80, t("else"))
    s.arrow(220, yb + 136, x + 110, yb + 170)
    s.arrow(570, yb + 136, x + w - 110, yb + 170)
    s.box(x, yb + 170, w, 46, t("reject_draw"), "draw < reject_prob → RandomEvent",
          tone="plain", size=12)
    s.arrow(x + w / 2, yb + 216, x + w / 2, yb + 246)
    s.text(x + w / 2, yb + 258, "OrderResult { status, status_info }", size=12, mono=True)
    return s


def bar_loop(t):
    s = Svg(1000, 470, t("bar_title"))
    s.text(500, 30, t("bar_head"), size=13, weight="600")
    # columns of a group
    cols = ["BTC", "ETH", "SOL"]
    for i, c in enumerate(cols):
        s.box(40, 70 + i * 60, 110, 40, c, "col " + str(i), size=12)
    s.box(210, 70, 230, 160, "", tone="ghost")
    s.text(325, 88, t("pass1"), size=12, weight="600")
    for i, k in enumerate(["price_rule", "val_rule", "stops_rule"]):
        s.text(228, 118 + i * 34, "• " + t(k), size=11, anchor="start")
    for i in range(3):
        s.arrow(150, 90 + i * 60, 210, 90 + i * 60)
    s.box(480, 90, 200, 50, t("group_value"), "cash + Σ pos × val_price", tone="accent", size=12)
    s.box(480, 170, 200, 50, "CallSeq", t("callseq_sub"), tone="accent", size=12, mono_sub=False)
    s.arrow(440, 115, 480, 115)
    s.arrow(580, 140, 580, 170)
    # pass 2
    s.box(720, 70, 240, 200, "", tone="ghost")
    s.text(840, 88, t("pass2"), size=12, weight="600")
    order = ["SOL (sell)", "BTC (buy)", "ETH (buy)"]
    for i, o in enumerate(order):
        s.box(745, 108 + i * 52, 190, 40, "execute_validated", o, tone="plain", size=11,
              mono_sub=False)
    s.arrow(680, 195, 720, 195)
    # sinks
    s.box(560, 320, 180, 46, "RecordSink", t("fills"), size=12, mono_sub=False)
    s.box(780, 320, 180, 46, "LogSink", t("considered"), size=12, mono_sub=False)
    s.arrow(840, 270, 650, 320)
    s.arrow(870, 270, 870, 320, head=True)
    s.text(40, 330, t("bar_note1"), size=11, color=MUTED, anchor="start")
    s.text(40, 352, t("bar_note2"), size=11, color=MUTED, anchor="start")
    s.text(40, 374, t("bar_note3"), size=11, color=MUTED, anchor="start")
    s.text(500, 440, t("bar_caption"), size=11, color=MUTED)
    return s


def signals(t):
    s = Svg(1000, 430, t("sig_title"))
    x, w, h = 60, 210, 50
    s.box(x, 40, w, h, t("stop_armed"), "sl_stop · sl_trail · tp_stop", tone="plain", size=12)
    s.box(x + 290, 40, 250, h, "stop_price()", t("gap_rule"), tone="accent", size=12,
          mono_sub=False)
    s.arrow(x + w, 65, x + 290, 65, t("yes"))
    s.box(x + 620, 40, 250, h, "Signals::from_stop", "StopExitPrice · StopExitMode", tone="warn",
          size=12)
    s.arrow(x + 540, 65, x + 620, 65, t("hit"))
    s.box(x + 290, 150, 250, h, "read_signals", t("two_conventions"), size=12, mono_sub=False)
    s.arrow(x + 415, 90, x + 415, 150, t("no_hit"), label_side="right")
    s.box(x + 290, 250, 250, 60, t("conflicts"),
          "signal → direction → opposite", size=12)
    s.arrow(x + 415, 200, x + 415, 250)
    s.box(x + 620, 250, 250, 60, "signals_to_size", "Amount · Value · Percent", tone="accent",
          size=12)
    s.arrow(x + 540, 280, x + 620, 280)
    s.path(f"M{x + 745},90 V250", color=LINE)
    s.box(x + 620, 350, 250, 44, "ValidatedOrder → " + t("kernel"), tone="ok", size=12)
    s.arrow(x + 745, 310, x + 745, 350)
    s.text(x, 180, t("trailing"), size=11, color=MUTED, anchor="start")
    s.text(x, 200, t("trailing2"), size=11, color=MUTED, anchor="start")
    s.text(x, 300, t("rearm"), size=11, color=MUTED, anchor="start")
    s.text(x, 320, t("rearm2"), size=11, color=MUTED, anchor="start")
    return s


def threads(t):
    s = Svg(780, 440, t("threads_title"))
    colors = [(ACCENT, ACCENT_BG), (OK, OK_BG), (PURPLE, PURPLE_BG)]
    chunks = [[0, 1, 2], [3, 4, 5], [6, 7]]
    s.text(40, 30, t("groups"), size=12, color=MUTED, anchor="start")
    bw = 210
    for th, groups in enumerate(chunks):
        c, bg = colors[th]
        x = 40 + th * 240
        for i, g in enumerate(groups):
            s.rect(x + i * 72, 46, 66, 36, bg, stroke=c, rx=4)
            s.text(x + i * 72 + 33, 64, f"G{g}", size=11, mono=True, color=c)
        s.brace(x + 2, x + len(groups) * 72 - 8, 90, f"chunk {th}", color=c)
        s.arrow(x + bw / 2, 112, x + bw / 2, 140, color=c)
        s.box(x, 140, bw, 50, t("thread") + f" {th}", t("own_sink"), tone="ghost", size=12,
              mono_sub=False)
        s.arrow(x + bw / 2, 190, x + bw / 2, 230, color=c)
        s.box(x, 230, bw, 50, f"buffer {th}", "orders · logs", size=12)
        s.arrow(x + bw / 2, 280, x + bw / 2, 330)
    s.box(40, 330, 690, 50, "concat in chunk order", t("rebase"), tone="accent", size=12,
          mono_sub=False)
    s.text(385, 414, t("threads_caption"), size=11, color=MUTED)
    return s


def rebuild(t):
    s = Svg(1080, 250, t("rebuild_title"))
    s.box(40, 100, 270, 50, "OrderRecords", "col · idx · size · price · fees · side", size=12)
    s.box(370, 40, 140, 46, "asset_flow", size=12)
    s.box(370, 160, 140, 46, "cash_flow", size=12)
    s.box(560, 40, 140, 46, "assets", t("per_symbol"), size=12, mono_sub=False)
    s.box(560, 160, 140, 46, "cash", t("per_group"), size=12, mono_sub=False)
    s.box(750, 100, 170, 50, "value", "cash + assets × close", tone="accent", size=12)
    s.box(950, 100, 100, 50, "returns", t("metrics"), tone="ok", size=12, mono_sub=False)
    s.arrow(310, 115, 370, 63)
    s.arrow(310, 135, 370, 183)
    s.arrow(510, 63, 560, 63)
    s.arrow(510, 183, 560, 183, "init_cash")
    s.arrow(700, 63, 750, 112)
    s.arrow(700, 183, 750, 138)
    s.arrow(920, 125, 950, 125)
    s.text(835, 30, "close", size=11, color=MUTED, mono=True)
    s.arrow(835, 40, 835, 100)
    s.text(540, 232, t("rebuild_caption"), size=11, color=MUTED)
    return s


def research(t):
    s = Svg(1000, 330, t("lab_title"))
    steps = [("propose", "LLM", "purple"), ("review", t("human"), "plain"),
             ("approve", "contract check", "plain"), ("run", t("in_sample"), "accent"),
             ("oos", t("batches"), "accent"), ("forward", t("shadow"), "accent")]
    for i, (a, b, tone) in enumerate(steps):
        x = 40 + i * 160
        s.box(x, 60, 145, 56, a, b, tone=tone, size=12, mono_sub=(a == "approve"))
        if i:
            s.arrow(x - 20, 88, x, 88)
    s.box(40, 200, 945, 50, "ledger.jsonl", t("ledger_sub"), tone="amber", size=12,
          mono_sub=False)
    for i in range(2, 6):
        x = 40 + i * 160 + 72
        s.arrow(x, 116, x, 200, "append" if i == 2 else None, label_side="right")
    s.path("M112,116 V200", color=WARN, dash=True, head=False)
    s.text(122, 160, t("never_results"), size=11, color=WARN, anchor="start")
    s.text(500, 290, t("lab_caption"), size=11, color=MUTED)
    return s


def verify(t):
    s = Svg(1000, 250, t("verify_title"))
    s.rect(24, 30, 472, 136, "none", stroke=LINE, rx=8, dash=True)
    s.text(40, 50, t("external_env"), size=11, color=MUTED, anchor="start")
    s.box(40, 90, 180, 56, "vectorbt 1.1.0 · Qlib", t("reference"), tone="ghost", size=12,
          mono_sub=False)
    s.box(280, 90, 200, 56, "tests/parity/generate_*.py", t("seeded"), size=11, mono_sub=False)
    s.box(540, 90, 180, 56, "tests/fixtures/*.tsv", t("committed"), tone="amber", size=11,
          mono_sub=False)
    s.box(780, 90, 180, 56, "cargo test", t("no_python"), tone="ok", size=12, mono_sub=False)
    s.arrow(220, 118, 280, 118, t("runs"))
    s.arrow(480, 118, 540, 118, t("writes"))
    s.arrow(720, 118, 780, 118, t("replays"))
    s.text(500, 200, t("verify_caption"), size=11, color=MUTED)
    s.text(500, 222, t("verify_caption2"), size=11, color=MUTED)
    return s


DIAGRAMS = {
    "pipeline": pipeline,
    "crates": crates,
    "columns": columns,
    "graph": graph,
    "evaluate": evaluate,
    "window": sliding,
    "cascade": cascade,
    "bar-loop": bar_loop,
    "signals": signals,
    "threads": threads,
    "rebuild": rebuild,
    "research": research,
    "verify": verify,
}

# ------------------------------------------------------------------ labels --

EN = {
    "pipeline_title": "The pipeline from local files to metrics",
    "per_factor": "(one Matrix per factor)",
    "files": "Local data", "panel": "Bar panel", "factors": "Factor matrices",
    "signals": "Entries / exits or target sizes", "sim": "Simulation", "records": "Records",
    "analysis": "Analysis",
    "edge_1": "load · decode · prepare", "edge_2": "compile · evaluate",
    "edge_3": "rule · delay · clean", "edge_4": "from_signals / from_orders",
    "edge_5": "RecordSink", "edge_6": "rebuild · metrics",
    "parallel_accounts": "parallel over accounts",
    "parallel_symbols": "parallel over symbols",
    "crates_title": "Workspace crates and their dependencies",
    "crates_note": "arrows point at the dependency",
    "facade": "re-exports all engine crates",
    "columns_title": "One column axis for parameters and symbols, stored column-major",
    "time": "time", "account": "account",
    "memory": "memory (one flat f64 buffer)",
    "contiguous": "one column is one contiguous slice",
    "graph_title": "Compiling a factor set into one deduplicated graph",
    "two_factors": "three factors", "compile": "compile",
    "hash_consing": "hash-consing", "hash_consing_sub": "same (op, inputs) → same node",
    "validation": "compile-time checks", "validation_sub": "windows, offsets, quantiles",
    "folding": "constant folding", "folding_sub": "-(1+2)*3 → -9",
    "order": "dependency order", "order_sub": "inputs always come first",
    "slots": "scratch slots", "slots_sub": "freed after the last reader",
    "graph_caption": "$close is read by five nodes but stored once; Alpha158: 357 nodes, 17 slots",
    "view": "view, no slot",
    "eval_title": "Native evaluation: each thread runs the whole graph for one symbol at a time",
    "inputs": "input fields (T × S)", "outputs": "outputs (one column slice per task)",
    "thread": "thread", "scratch": "scratch: graph.slots series of length T",
    "eval_caption": "rayon into_par_iter over columns; disjoint column slices, no locks, one parallel region",
    "window_title": "Incremental rolling window",
    "window_head": "Each row: push the entering value, pop the leaving one — O(1)",
    "window_n": "window N = 5", "pop": "pop", "push": "push", "accumulator": "accumulator",
    "rebuild": "rebuild every N rows", "rebuild_sub": "rounding drift is bounded to two windows",
    "shift": "shifted sums", "shift_sub": "offsets from a value in the window",
    "constant": "exact constant windows", "constant_sub": "identical values → variance exactly 0",
    "nan": "NaN occupies a slot", "nan_sub": "but adds nothing to the sums",
    "cascade_title": "The size cascade inside execute_order",
    "ignored_rejected": "not filled", "dispatch": "sign of size",
    "reject_draw": "random rejection (optional)",
    "buy_sub": "slippage · fees inside the cash cap · granularity",
    "sell_sub": "slippage · short collateral · granularity", "else": "else (0, NaN)",
    "bar_title": "One bar of a shared-cash account",
    "bar_head": "Per bar, per account group: resolve everything, then execute in call order",
    "pass1": "Pass 1 — every column", "price_rule": "price: +inf → close",
    "val_rule": "val_price: ffill", "stops_rule": "stops (from_signals)",
    "group_value": "bar-open value", "callseq_sub": "Auto: sells before buys",
    "pass2": "Pass 2 — in call order", "fills": "fills", "considered": "every order considered",
    "bar_note1": "Every order is sized against the bar-open value;",
    "bar_note2": "it is threaded through the bar, not re-summed.",
    "bar_note3": "Ungrouped: each column is its own account.",
    "bar_caption": "One thread owns the whole group; different groups run on different threads",
    "sig_title": "What from_signals decides for one column at one bar",
    "stop_armed": "stops armed?", "gap_rule": "gap → open, touch → level", "yes": "yes",
    "hit": "hit", "no_hit": "no hit / not armed", "two_conventions": "entries/exits or 4 arrays",
    "conflicts": "resolve conflicts", "kernel": "kernel",
    "trailing": "Trailing anchor moves to high/low",
    "trailing2": "after the test, never before.",
    "rearm": "After a fill: flat → clear stops;",
    "rearm2": "open/reverse → re-anchor; add → StopUpdateMode.",
    "threads_title": "Parallel simulation: contiguous chunks of account groups",
    "groups": "account groups", "own_sink": "owns its sink and counters",
    "rebase": "re-base ids and order_id",
    "threads_caption": "Same output for any thread count; the lowest chunk's error wins",
    "rebuild_title": "Balances are rebuilt from order records alone",
    "per_symbol": "per symbol", "per_group": "per group", "metrics": "metrics · stats",
    "rebuild_caption": "Each series is derived lazily and cached once (OnceLock) in Portfolio",
    "lab_title": "The pre-registered research protocol",
    "human": "human", "in_sample": "once, in-sample", "batches": "≤ 3 batches, 90 d apart",
    "shadow": "shadow tracking",
    "ledger_sub": "append-only · SHA-256 chained · git-committed · counts N for Bonferroni",
    "never_results": "the LLM never sees results",
    "lab_caption": "Failed runs still count toward N: the data was read",
    "verify_title": "Differential testing against committed fixtures",
    "reference": "reference", "seeded": "edge cases + seeded random",
    "committed": "committed", "no_python": "no Python needed", "runs": "runs",
    "writes": "writes", "replays": "replays",
    "external_env": "external environment (.venv-parity, conda)",
    "verify_caption": "Records are compared field by field: floats at 1e-9 / 1e-12, integers exactly",
    "verify_caption2": "A changed line after regeneration is a finding, not noise",
}

ZH = {
    "pipeline_title": "从本地文件到指标的流水线",
    "per_factor": "（每个因子一个 Matrix）",
    "files": "本地数据", "panel": "K 线面板", "factors": "因子矩阵",
    "signals": "入场/离场或目标仓位", "sim": "模拟", "records": "记录", "analysis": "分析",
    "edge_1": "加载 · 解码 · prepare", "edge_2": "编译 · 求值", "edge_3": "规则 · 延迟 · 清洗",
    "edge_4": "from_signals / from_orders", "edge_5": "RecordSink", "edge_6": "重建 · 指标",
    "parallel_accounts": "按账户并行", "parallel_symbols": "按标的并行",
    "crates_title": "工作区 crate 及其依赖", "crates_note": "箭头指向被依赖的 crate",
    "facade": "重新导出所有引擎 crate",
    "columns_title": "参数和标的共用一条列轴，按列主序存储",
    "time": "时间", "account": "账户", "memory": "内存（一整块 f64 缓冲区）",
    "contiguous": "一列就是一段连续切片",
    "graph_title": "把因子集编译成一张去重后的依赖图",
    "two_factors": "三个因子", "compile": "编译",
    "hash_consing": "哈希合并", "hash_consing_sub": "相同 (op, 输入) → 同一节点",
    "validation": "编译期校验", "validation_sub": "窗口、偏移、分位数",
    "folding": "常量折叠", "folding_sub": "-(1+2)*3 → -9",
    "order": "依赖顺序", "order_sub": "输入总在前面",
    "slots": "暂存槽位", "slots_sub": "最后一个读者之后复用；输出直接写",
    "graph_caption": "$close 被五个节点读取但只存一份；Alpha158：357 个节点、17 个槽位",
    "view": "视图，不占槽位",
    "eval_title": "原生求值：每个线程一次对一个标的跑完整张图",
    "inputs": "输入字段（T × S）", "outputs": "输出（每个任务一段列切片）",
    "thread": "线程", "scratch": "暂存：graph.slots 条长度为 T 的序列",
    "eval_caption": "rayon 按列 into_par_iter；列切片互不重叠，无锁，只有一个并行区域",
    "window_title": "增量滚动窗口",
    "window_head": "每一行：加入进入的值，移除离开的值——O(1)",
    "window_n": "窗口 N = 5", "pop": "移除", "push": "加入", "accumulator": "累加器",
    "rebuild": "每 N 行重建", "rebuild_sub": "舍入漂移最多累积两个窗口",
    "shift": "平移求和", "shift_sub": "以窗口内某个值为基准",
    "constant": "精确的常数窗口", "constant_sub": "值全相同 → 方差精确为 0",
    "nan": "NaN 占位", "nan_sub": "但不参与求和",
    "cascade_title": "execute_order 内的数量换算链",
    "ignored_rejected": "不成交", "dispatch": "按 size 的符号",
    "reject_draw": "随机拒单（可选）",
    "buy_sub": "滑点 · 手续费计入现金上限 · 粒度",
    "sell_sub": "滑点 · 空头保证金 · 粒度", "else": "否则（0、NaN）",
    "bar_title": "共享现金账户的一根 K 线",
    "bar_head": "每根 K 线、每个账户组：先把一切确定下来，再按调用顺序执行",
    "pass1": "第一遍——每一列", "price_rule": "价格：+inf → 收盘价",
    "val_rule": "估值价：向前填充", "stops_rule": "止损止盈（from_signals）",
    "group_value": "开盘净值", "callseq_sub": "Auto：先卖后买",
    "pass2": "第二遍——按调用顺序", "fills": "成交", "considered": "每笔被考虑的订单",
    "bar_note1": "每笔订单都以开盘净值计算数量；",
    "bar_note2": "净值在整根 K 线内传递，不重新求和。",
    "bar_note3": "不分组时：每列自己就是一个账户。",
    "bar_caption": "一个线程负责整个组；不同的组在不同线程上运行",
    "sig_title": "from_signals 在一根 K 线上为一列做的决定",
    "stop_armed": "止损已布置？", "gap_rule": "跳空 → 开盘价，触及 → 触发价", "yes": "是",
    "hit": "触发", "no_hit": "未触发 / 未布置", "two_conventions": "entries/exits 或 4 个数组",
    "conflicts": "解决冲突", "kernel": "内核",
    "trailing": "跟踪止损的锚点更新为最高/最低价，",
    "trailing2": "在检查之后，而不是之前。",
    "rearm": "成交后：空仓 → 清除；",
    "rearm2": "开仓/反手 → 重新设锚；加仓 → StopUpdateMode。",
    "threads_title": "并行模拟：账户组切成连续的块",
    "groups": "账户组", "own_sink": "拥有自己的 sink 和计数器",
    "rebase": "重新定基 id 和 order_id",
    "threads_caption": "线程数不影响输出；多个块出错时取序号最小的块",
    "rebuild_title": "余额只由订单记录重建",
    "per_symbol": "按标的", "per_group": "按组", "metrics": "指标 · 统计",
    "rebuild_caption": "每条序列都在 Portfolio 中惰性推导，并只缓存一次（OnceLock）",
    "lab_title": "预注册的研究协议",
    "human": "人工", "in_sample": "样本内，只跑一次", "batches": "≤ 3 批，间隔 90 天",
    "shadow": "影子跟踪",
    "ledger_sub": "只追加 · SHA-256 链 · git 提交 · 统计 Bonferroni 的 N",
    "never_results": "LLM 永远看不到结果",
    "lab_caption": "失败的运行也计入 N：数据已经被读过了",
    "verify_title": "与已提交的测试夹具做差分测试",
    "reference": "参考实现", "seeded": "边界情况 + 固定种子随机",
    "committed": "提交进仓库", "no_python": "不需要 Python", "runs": "运行",
    "writes": "写出", "replays": "回放",
    "external_env": "外部环境（.venv-parity、conda）",
    "verify_caption": "记录逐字段比较：浮点 1e-9 / 1e-12，整数精确相等",
    "verify_caption2": "重新生成后出现变动的行是一个发现，而不是噪声",
}


def main() -> None:
    assert EN.keys() == ZH.keys(), set(EN) ^ set(ZH)
    problems = []
    for lang, labels in (("en", EN), ("zh", ZH)):
        out = os.path.join(ROOT, lang)
        os.makedirs(out, exist_ok=True)
        for name, draw in DIAGRAMS.items():
            drawing = draw(labels.__getitem__)
            problems += [f"{lang}/{name}: {p}" for p in drawing.problems]
            svg = drawing.render()
            with open(os.path.join(out, f"{name}.svg"), "w", encoding="utf-8") as f:
                f.write(svg)
    if problems:
        raise SystemExit("layout problems:\n  " + "\n  ".join(problems))
    print(f"wrote {len(DIAGRAMS)} diagrams × 2 languages to {ROOT}")


if __name__ == "__main__":
    main()
