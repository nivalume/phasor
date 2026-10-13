import { describe, expect, it } from "vitest";
import { buildWiki } from "../core/wiki.js";
import { search } from "./search";

const page = (path: string, title: string, body: string) => ({
  path,
  text: `---\ntitle: ${title}\nkind: concept\nsummary: s\nparent: x\n---\n# ${title}\n\n${body}\n`,
});

describe("search", () => {
  const wiki = buildWiki([
    page("wiki/content/concepts/a.md", "空头保证金", "## 记账\n\n`lock_cash` 打开时预算从 `free_cash` 起算。"),
    page("wiki/content/concepts/b.md", "lock_cash 预算", "正文。"),
    page("wiki/content/concepts/c.md", "无关", "什么都没有。"),
  ]);
  const pages = [...wiki.pages.values()];

  it("matches identifiers with underscores in the body, not only in titles", () => {
    expect(search(pages, "lock_cash").map((h) => h.page.id)).toEqual(["b", "a"]);
  });
  it("points a body hit at the section it is in", () => {
    const hit = search(pages, "free_cash")[0];
    expect(hit.page.id).toBe("a");
    expect(hit.anchor).toBe("记账");
    expect(hit.snippet).toContain("free_cash");
  });
  it("requires every term and matches Chinese substrings", () => {
    expect(search(pages, "保证金 lock_cash").map((h) => h.page.id)).toEqual(["a"]);
    expect(search(pages, "没有")).toHaveLength(1);
    expect(search(pages, "   ")).toEqual([]);
  });
});
