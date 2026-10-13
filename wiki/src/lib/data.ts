import { buildWiki, type Wiki, type Page } from "../core/wiki.js";

// Every page, as raw text, keyed by its path relative to this file.
const rawPages = import.meta.glob("../../content/**/*.md", { query: "?raw", import: "default", eager: true }) as Record<string, string>;

// Diagrams the pages embed from docs/img (the architecture SVGs).
const rawImages = import.meta.glob("../../../docs/img/**/*.svg", { query: "?url", import: "default", eager: true }) as Record<string, string>;

const files = Object.entries(rawPages).map(([key, text]) => ({
  path: `wiki/${key.replace(/^(\.\.\/)+/, "")}`,
  text,
}));

export const wiki: Wiki = buildWiki(files);

/** Repo-relative image path (docs/img/zh/x.svg) → bundled URL. */
export const imageUrls = new Map<string, string>(
  Object.entries(rawImages).map(([key, url]) => [key.replace(/^(\.\.\/)+/, ""), url]),
);

/** Pages sorted for navigation: by kind order, then `order`, then title. */
export function sortedPages(pages: Iterable<Page>): Page[] {
  const kindRank: Record<string, number> = { guide: 0, crate: 1, concept: 2, algorithm: 3, reference: 4, tooling: 5 };
  return [...pages].sort(
    (a, b) =>
      (kindRank[a.kind] ?? 9) - (kindRank[b.kind] ?? 9) ||
      a.order - b.order ||
      a.title.localeCompare(b.title, "zh-CN"),
  );
}

export function getPage(id: string): Page | undefined {
  return wiki.pages.get(id);
}

/** Children of a crate/tooling page (pages whose parent is it), in order. */
export function childrenOf(id: string): Page[] {
  return sortedPages([...wiki.pages.values()].filter((p) => p.parent === id));
}
