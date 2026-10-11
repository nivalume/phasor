# Mermaid syntax pitfalls (GitHub / mermaid 11)

Each entry: what breaks → what the parser says → the fix. The first block of
entries are failures actually hit in this repository; the rest are the usual
suspects.

## Hit in this repository

| What breaks | Parser message | Fix |
|---|---|---|
| `A --> B : FactorInputs::from_ohlcv` (classDiagram relation label with `::`) | `Expecting 'NEWLINE', 'EOF', got 'COLON'` | `FactorInputs.from_ohlcv` — no colon of any kind inside a relation label |
| `Pair{corr, window}` inside a class body | `Expecting 'STRUCT_STOP', 'MEMBER', got 'OPEN_IN_STRUCT'` | `Pair(corr, window)` or `Pair corr window` — braces end the class body |
| `Scalar(T)` as an enum variant in a class body | renders in the *method* compartment | cosmetic; drop the parentheses if it should be an attribute |

## Class diagrams

- Generics: `Vec~f64~`, `Option~usize~`, `HashMap~String, &[f64]~`. Never `<`/`>`.
- Lifetimes and `&` are fine inside `~ ~` and in member text: `Option~&'static str~`.
- A member line is `visibility name` or `visibility name(args) ReturnType`; a
  trailing `$` marks static. Chinese parentheses（…）are ordinary text and safe.
- Several members on one line: `class X { +a; +b; +c() }`.
- Stereotype on its own line: `<<enum>>`; or inline `class X { <<trait>> +m() }`.
- Relation syntax: `A <|-- B` (inherit), `A *-- B` (compose), `A o-- B`
  (aggregate), `A --> B` (assoc), `A ..> B` (dependency), `A <|.. B`
  (realize/implement). Label after ` : `, no colons inside, `×4` and `→` are fine.
- `direction LR` / `TB` right after `classDiagram`.
- No `<br/>` in members; no `%%` comments inside a class body.

## Flowcharts

- Quote every label: `A["text"]`, `B{"question?"}`, `C(("round"))`.
  Unquoted `(`, `)`, `[`, `]`, `{`, `}`, `|`, `"` break the node.
- `<br/>` for line breaks only inside quoted labels.
- Node ids: ASCII letters/digits/underscore; never `end` (reserved); avoid
  starting with a digit; do not reuse an id as an edge label.
- Edge labels: `A -- 是 --> B` or `A -->|是| B`; a label containing `|` needs
  the `-- … -->` form.
- `flowchart`, not `graph`, when using `classDef`/`class` with subgraphs.
- `classDef name fill:#hex,stroke:#hex;` then `class A,B name;` — ids
  separated by commas, no spaces after the commas.
- `subgraph id ["title"] … end`; nodes inside may still be linked from outside.
- A literal `#` in a label: use `#35;`; a literal quote: `#quot;`.

## Sequence diagrams

- `participant P as 显示名` first; messages `A->>B: text`, returns `A-->>B:`.
- `Note over A,B: text` (comma, no space before B is safest).
- Blocks: `loop … end`, `alt … else … end`, `opt … end`, `par … and … end`.
- A message text may contain `:`? Only the first `:` separates; later ones are
  text, but `;` is not. Avoid `;` in message text.
- Participants are created in order of first mention unless declared.

## State diagrams

- `stateDiagram-v2`; `[*]` for start/end; `A --> B : label` with no colon in
  the label; state ids ASCII, display names via `state "显示名" as id`.

## Gantt

- Requires `dateFormat`; `D` (day number) works for abstract plans.
- Each task: `name :id, start, duration` or `name :id, after otherId, 2d`.
- `section` lines group tasks; `title` is optional.

## General

- Fence exactly as ```mermaid (lower case) on its own line; close with ```.
- No tabs inside the block; two-space indentation renders everywhere.
- Full-width punctuation (：，（）) and emoji are safe inside quoted labels
  and in class members; `✓ ◐ ✗ → ← ×` were used in this repo without issues.
- GitHub renders Mermaid 11; `mindmap`, `timeline` and `xychart` are
  supported there but not in every other viewer — prefer the five types above
  for portability.
- Run `python3 skills/mermaid-diagrams/scripts/check_mermaid.py FILE` after
  every edit; the message it prints is the parser's own.
