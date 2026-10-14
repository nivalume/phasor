import { describe, expect, it } from "vitest";
import {
  buildWiki,
  extractHeadings,
  extractInlineCode,
  maskCode,
  neighbourhood,
  parseSourceRef,
  resolveCodePath,
  resolveHref,
  splitFrontmatter,
} from "./wiki.js";

const page = (path: string, front: string, body: string) => ({ path, text: `---\n${front}\n---\n${body}` });

describe("resolveHref", () => {
  const from = "wiki/content/algorithms/buy-leg.md";
  it("resolves sibling and cross-section page links", () => {
    expect(resolveHref(from, "../concepts/short-margin.md")).toEqual({ type: "page", id: "short-margin", anchor: null });
    expect(resolveHref(from, "../algorithms/sell-leg.md#思路")).toEqual({ type: "page", id: "sell-leg", anchor: "思路" });
  });
  it("resolves links from index.md", () => {
    expect(resolveHref("wiki/content/index.md", "crates/phasor-exec.md")).toEqual({ type: "page", id: "phasor-exec", anchor: null });
  });
  it("treats other repository paths as repo links", () => {
    expect(resolveHref(from, "../../../docs/plan.md")).toEqual({ type: "repo", path: "docs/plan.md", anchor: null });
    expect(resolveHref(from, "../../../docs/img/zh/bar-loop.svg")).toEqual({ type: "repo", path: "docs/img/zh/bar-loop.svg", anchor: null });
  });
  it("keeps external URLs and in-page anchors", () => {
    expect(resolveHref(from, "https://example.com/x")).toEqual({ type: "external", url: "https://example.com/x" });
    expect(resolveHref(from, "#位置")).toEqual({ type: "anchor", anchor: "位置" });
  });
  it("rejects absolute paths and paths leaving the repository", () => {
    expect(resolveHref(from, "/docs/plan.md").type).toBe("invalid");
    expect(resolveHref(from, "../../../../outside.md").type).toBe("invalid");
  });
});

describe("headings", () => {
  it("slugs like GitHub, keeping Chinese and numbering duplicates", () => {
    const h = extractHeadings("# 标题\n\n## 与 Qlib 的对应\n\n## `buy`：买入\n\n## 相关\n\n## 相关\n\n```\n## not a heading\n```\n");
    expect(h.map((x) => x.slug)).toEqual(["标题", "与-qlib-的对应", "buy买入", "相关", "相关-1"]);
  });
});

describe("maskCode and inline code", () => {
  it("blanks code but keeps line numbers", () => {
    const body = "a `[x](y.md)` b\n```\n[z](w.md)\n```\nc";
    const masked = maskCode(body);
    expect(masked.split("\n")).toHaveLength(5);
    expect(masked).not.toContain("y.md");
    expect(masked).not.toContain("w.md");
  });
  it("lists inline code outside fences with file lines", () => {
    const code = extractInlineCode("x `crates/a.rs:3`\n```\n`skip`\n```\n`src/lib.rs`", 10);
    expect(code).toEqual([
      { text: "crates/a.rs:3", line: 10 },
      { text: "src/lib.rs", line: 14 },
    ]);
  });
});

describe("resolveCodePath", () => {
  const files = new Set(["crates/phasor-sim/src/lib.rs", "crates/phasor-sim/tests/parity_signals.rs", "crates/phasor-py/src/array.rs", "tests/fixtures/a.tsv"]);
  const exists = (p: string) => files.has(p) || [...files].some((f) => f.startsWith(`${p}/`));
  const sim = { kind: "crate", id: "phasor-sim", parent: null } as never;
  const algo = { kind: "algorithm", id: "bar-loop", parent: "phasor-sim" } as never;
  it("keeps paths from the repository root", () => {
    expect(resolveCodePath("tests/fixtures/a.tsv", null, exists)).toBe("tests/fixtures/a.tsv");
    expect(resolveCodePath("tests/fixtures/", null, exists)).toBe("tests/fixtures/");
  });
  it("resolves paths relative to the page's crate, keeping the line suffix", () => {
    expect(resolveCodePath("src/lib.rs:57-93", sim, exists)).toBe("crates/phasor-sim/src/lib.rs:57-93");
    expect(resolveCodePath("tests/parity_signals.rs", algo, exists)).toBe("crates/phasor-sim/tests/parity_signals.rs");
  });
  it("resolves crate-qualified paths under crates/", () => {
    expect(resolveCodePath("phasor-py/src/array.rs", null, exists)).toBe("crates/phasor-py/src/array.rs");
  });
  it("returns null for missing files and non-paths", () => {
    expect(resolveCodePath("src/missing.rs", sim, exists)).toBeNull();
    expect(resolveCodePath("size_type", sim, exists)).toBeNull();
    expect(resolveCodePath("a / b", sim, exists)).toBeNull();
  });
});

describe("buildWiki", () => {
  const wiki = buildWiki([
    page("wiki/content/crates/phasor-exec.md", "title: exec\nkind: crate\nsummary: s\ndepends: [phasor-core]", "# exec\n\n[buy](../algorithms/buy-leg.md)\n"),
    page("wiki/content/crates/phasor-core.md", "title: core\nkind: crate\nsummary: s", "# core\n"),
    page(
      "wiki/content/algorithms/buy-leg.md",
      "title: buy\nkind: algorithm\nsummary: s\nparent: phasor-exec\nrelated: [phasor-core]",
      "# buy\n\n见 [exec](../crates/phasor-exec.md) 与 `[not](../crates/phasor-core.md)`\n",
    ),
  ]);
  it("builds one page per file", () => {
    expect([...wiki.pages.keys()].sort()).toEqual(["buy-leg", "phasor-core", "phasor-exec"]);
    expect(wiki.problems).toEqual([]);
  });
  it("creates typed edges from links and frontmatter", () => {
    const key = (e: { source: string; target: string; kind: string }) => `${e.kind}:${e.source}>${e.target}`;
    expect(wiki.edges.map(key).sort()).toEqual(
      ["depends:phasor-exec>phasor-core", "link:phasor-exec>buy-leg", "link:buy-leg>phasor-exec", "parent:buy-leg>phasor-exec", "related:buy-leg>phasor-core"].sort(),
    );
  });
  it("indexes backlinks and neighbourhoods", () => {
    expect(wiki.incoming.get("phasor-exec")?.map((e) => e.source).sort()).toEqual(["buy-leg", "buy-leg"]);
    expect([...neighbourhood(wiki, "phasor-core", 1)].sort()).toEqual(["buy-leg", "phasor-core", "phasor-exec"]);
  });
  it("reports duplicate ids and broken frontmatter", () => {
    const bad = buildWiki([
      page("wiki/content/guides/x.md", "title: a\nkind: guide\nsummary: s", ""),
      page("wiki/content/reference/x.md", "title: b\nkind: reference\nsummary: s", ""),
      { path: "wiki/content/guides/y.md", text: "no frontmatter" },
    ]);
    const messages = bad.problems.map((p) => p.message).join("\n");
    expect(messages).toMatch(/duplicate page id "x"/);
    expect(messages).toMatch(/missing frontmatter/);
  });
});

describe("small helpers", () => {
  it("parses source references", () => {
    expect(parseSourceRef("crates/a.rs:12-30")).toEqual({ path: "crates/a.rs", start: 12, end: 30 });
    expect(parseSourceRef("crates/a.rs:7")).toEqual({ path: "crates/a.rs", start: 7, end: null });
    expect(parseSourceRef("docs/plan.md")).toEqual({ path: "docs/plan.md", start: null, end: null });
  });
  it("splits frontmatter and reports where the body starts", () => {
    const { data, body, bodyLine, error } = splitFrontmatter("---\ntitle: t\nkind: guide\n---\n# t\n");
    expect(error).toBeNull();
    expect(data).toEqual({ title: "t", kind: "guide" });
    expect(body).toBe("# t\n");
    expect(bodyLine).toBe(5);
  });
});
