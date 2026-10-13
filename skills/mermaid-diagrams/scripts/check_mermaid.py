#!/usr/bin/env python3
"""Extract every ```mermaid block from Markdown files and render each one with
mermaid-cli, so a diagram that GitHub would refuse to draw fails here first.

Usage
    python3 skills/mermaid-diagrams/scripts/check_mermaid.py [PATH ...]
        PATH is a Markdown file or a directory (searched recursively, *.md).
        Default: docs/ and wiki/content/

Options
    --out DIR        keep rendered SVGs in DIR (default: a temporary directory)
    --png            also render PNGs (slower; useful to look at a diagram)
    --width N        PNG width in pixels (default 1600)
    --list           only list the blocks found, do not render
    --mmdc PATH      mermaid-cli binary (default: $MMDC, node_modules/.bin/mmdc
                     found upward from the cwd, `mmdc` on PATH, else
                     `npx -y @mermaid-js/mermaid-cli@11.4.2`)
    --chrome PATH    Chromium/Chrome binary for puppeteer (default:
                     $PUPPETEER_EXECUTABLE_PATH, then common locations)
    --keep-going     render every block even after a failure (default: on)

Exit status: 0 when every block renders, 1 when any fails, 2 on setup errors.

Only the Python standard library is used. mermaid-cli needs Node.js and a
Chromium; see ../SKILL.md for the one-time setup.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

FENCE = re.compile(r"^```mermaid[ \t]*\n(.*?)^```[ \t]*$", re.S | re.M)

CHROME_CANDIDATES = [
    "/opt/pw-browsers/chromium-*/chrome-linux/chrome",
    "/opt/pw-browsers/chromium_headless_shell-*/chrome-linux/headless_shell",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    os.path.expanduser("~/.cache/puppeteer/chrome/*/chrome-linux64/chrome"),
    os.path.expanduser("~/.cache/ms-playwright/chromium-*/chrome-linux/chrome"),
]


@dataclass
class Block:
    file: Path
    index: int
    line: int  # 1-based line of the ```mermaid fence
    source: str

    @property
    def kind(self) -> str:
        for raw in self.source.splitlines():
            text = raw.strip()
            if text and not text.startswith("%%"):
                return text.split()[0]
        return "?"

    @property
    def stem(self) -> str:
        return f"{self.file.stem}-{self.index}"


def find_markdown(paths: list[str]) -> list[Path]:
    out: list[Path] = []
    for raw in paths or ["docs", "wiki/content"]:
        p = Path(raw)
        if p.is_dir():
            out.extend(sorted(q for q in p.rglob("*.md") if "node_modules" not in q.parts))
        elif p.is_file():
            out.append(p)
        else:
            print(f"warning: {raw} is neither a file nor a directory", file=sys.stderr)
    return out


def extract(md: Path) -> list[Block]:
    text = md.read_text(encoding="utf-8")
    blocks = []
    for i, m in enumerate(FENCE.finditer(text)):
        line = text.count("\n", 0, m.start()) + 1
        blocks.append(Block(md, i, line, m.group(1)))
    return blocks


def find_mmdc(explicit: str | None) -> list[str]:
    if explicit:
        return [explicit]
    env = os.environ.get("MMDC")
    if env:
        return [env]
    here = Path.cwd().resolve()
    for d in [here, *here.parents]:
        cand = d / "node_modules" / ".bin" / ("mmdc.cmd" if os.name == "nt" else "mmdc")
        if cand.exists():
            return [str(cand)]
    on_path = shutil.which("mmdc")
    if on_path:
        return [on_path]
    npx = shutil.which("npx")
    if npx:
        return [npx, "-y", "@mermaid-js/mermaid-cli@11.4.2"]
    return []


def find_chrome(explicit: str | None) -> str | None:
    if explicit:
        return explicit
    env = os.environ.get("PUPPETEER_EXECUTABLE_PATH")
    if env:
        return env
    for pattern in CHROME_CANDIDATES:
        hits = sorted(glob.glob(pattern))
        if hits:
            return hits[-1]
    return None


def parse_error(stderr: str) -> str:
    """The one line a reader needs: mermaid's parse error with its position."""
    lines = stderr.splitlines()
    for i, line in enumerate(lines):
        if "Parse error" in line or "Syntax error" in line or "UnknownDiagramError" in line:
            snippet = lines[i : i + 4]
            return "\n      ".join(s.rstrip() for s in snippet if s.strip())
    for line in lines:
        if line.startswith("Error:"):
            return line.strip()
    tail = [s for s in lines if s.strip()][-3:]
    return "\n      ".join(tail) if tail else "(no output)"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("paths", nargs="*")
    ap.add_argument("--out")
    ap.add_argument("--png", action="store_true")
    ap.add_argument("--width", type=int, default=1600)
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--mmdc")
    ap.add_argument("--chrome")
    ap.add_argument("--keep-going", action="store_true", default=True)
    args = ap.parse_args()

    files = find_markdown(args.paths)
    blocks = [b for f in files for b in extract(f)]
    if not blocks:
        print("no ```mermaid blocks found")
        return 0
    print(f"{len(blocks)} mermaid block(s) in {len(files)} file(s)")
    if args.list:
        for b in blocks:
            print(f"  {b.file}:{b.line}  #{b.index}  {b.kind}")
        return 0

    mmdc = find_mmdc(args.mmdc)
    if not mmdc:
        print("error: mermaid-cli not found; install with `npm install @mermaid-js/mermaid-cli@11.4.2` "
              "or pass --mmdc", file=sys.stderr)
        return 2
    chrome = find_chrome(args.chrome)

    out_dir = Path(args.out) if args.out else Path(tempfile.mkdtemp(prefix="mermaid-check-"))
    out_dir.mkdir(parents=True, exist_ok=True)
    puppeteer = out_dir / "puppeteer.json"
    cfg = {"args": ["--no-sandbox", "--disable-setuid-sandbox", "--disable-gpu"]}
    if chrome:
        cfg["executablePath"] = chrome
    puppeteer.write_text(json.dumps(cfg), encoding="utf-8")
    print(f"mmdc: {' '.join(mmdc)}")
    print(f"chrome: {chrome or '(puppeteer default; set --chrome or PUPPETEER_EXECUTABLE_PATH if it fails)'}")
    print(f"output: {out_dir}")

    failures = 0
    for b in blocks:
        src = out_dir / f"{b.stem}.mmd"
        src.write_text(b.source, encoding="utf-8")
        targets = [out_dir / f"{b.stem}.svg"]
        if args.png:
            targets.append(out_dir / f"{b.stem}.png")
        ok = True
        for target in targets:
            cmd = [*mmdc, "-q", "-p", str(puppeteer), "-i", str(src), "-o", str(target), "-b", "white"]
            if target.suffix == ".png":
                cmd += ["-w", str(args.width)]
            proc = subprocess.run(cmd, capture_output=True, text=True)
            if proc.returncode != 0 or not target.exists():
                ok = False
                failures += 1
                print(f"FAIL  {b.file}:{b.line}  block #{b.index} ({b.kind})")
                print(f"      {parse_error(proc.stderr + proc.stdout)}")
                break
        if ok:
            print(f"ok    {b.file}:{b.line}  block #{b.index} ({b.kind})")
    print(f"{len(blocks) - failures}/{len(blocks)} rendered")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
