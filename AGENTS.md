# AGENTS.md

Instructions for coding agents (Codex, Grok Build, Claude Code via `CLAUDE.md`, and any tool that reads this file).

## What this repository is

phasor: a parallel, factor-driven vectorized backtesting engine in Rust with a Python package.
Read `README.md`, then `docs/ARCHITECTURE.md` (design, bilingual) and the source knowledge base
(https://phasor-docs.vercel.app, repository nivalume/phasor-docs: how to read the code, Chinese) before changing anything non-trivial.
`docs/plan.md` is the task list.

## Commands

```bash
cargo test --workspace                                   # all Rust tests; replays committed parity fixtures, no Python needed
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
uv sync && uv run --with pytest python -m pytest crates/phasor-py/tests -q   # Python package
python docs/img/generate.py                              # regenerate the architecture SVGs
```

## Rules that are easy to break

- `tests/fixtures/*.tsv` are outputs of the reference implementations; never regenerate them to make a change pass.
- Financial comparisons go through `phasor_core::math` (`is_close`, `add`), never `==` / `<=` on floats; `clippy::float_cmp` is denied.
- Serial and threaded runs must stay bit-identical; random draws are functions of `(seed, row, col)`.
- Library crates never download or write files; network code lives in `examples/`.
- Analysis rebuilds balances from records only: anything that changes cash or a position must be a record.
- The knowledge base (nivalume/phasor-docs) cites code as links to one pinned commit of this repository, with
  line numbers checked against it; when code a page describes changes, update the page there (see its README).

## Skills

Reusable skills live in `skills/` (Agent Skills format, one `SKILL.md` per directory) and are linked into
`.claude/skills/`, `.agents/skills/` and `.grok/skills/` so every tool discovers them.

| Skill | When to use |
|---|---|
| `skills/docs-canvas` | Docs overview / architecture walkthrough / API reference as a navigable Cursor Canvas; requires Cursor Canvas SDK. |
| `skills/mermaid-diagrams` | Any Markdown with a ```mermaid block: pick the diagram type, follow the house style, then run `python3 skills/mermaid-diagrams/scripts/check_mermaid.py <file-or-dir>` and fix every `FAIL` before committing. |

Codex: `$docs-canvas`, `$mermaid-diagrams`, or `/skills`. Grok Build: `grok inspect` lists what was discovered. Claude Code: loads it automatically.
