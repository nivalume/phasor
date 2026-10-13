import type { Page } from "../core/wiki.js";

export interface Hit {
  page: Page;
  score: number;
  /** A line of the body around the first match, or the summary. */
  snippet: string;
  /** Heading slug of the section the snippet came from, if any. */
  anchor: string | null;
}

/** Strip Markdown syntax enough for snippets and matching. Underscores stay: they are part of identifiers like lock_cash. */
function plain(line: string): string {
  return line
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[`*>#|]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

interface Indexed {
  page: Page;
  title: string;
  id: string;
  tags: string;
  summary: string;
  headings: { text: string; slug: string }[];
  lines: { text: string; anchor: string | null }[];
}

let cache: Indexed[] | null = null;
let cacheKey: Page[] | null = null;

function index(pages: Page[]): Indexed[] {
  if (cache && cacheKey === pages) return cache;
  cache = pages.map((page) => {
    const lines: { text: string; anchor: string | null }[] = [];
    let anchor: string | null = null;
    let headingIdx = 0;
    let fence = false;
    for (const raw of page.body.split("\n")) {
      if (/^\s*(```|~~~)/.test(raw)) {
        fence = !fence;
        continue;
      }
      if (!fence && /^#{1,6}\s/.test(raw)) {
        anchor = page.headings[headingIdx]?.slug ?? anchor;
        headingIdx++;
        continue;
      }
      const t = plain(raw);
      if (t.length > 1) lines.push({ text: t, anchor });
    }
    return {
      page,
      title: page.title.toLowerCase(),
      id: page.id.toLowerCase(),
      tags: page.tags.join(" ").toLowerCase(),
      summary: page.summary.toLowerCase(),
      headings: page.headings.map((h) => ({ text: h.text.toLowerCase(), slug: h.slug })),
      lines,
    };
  });
  cacheKey = pages;
  return cache;
}

function clip(text: string, at: number, width = 90): string {
  const start = Math.max(0, at - Math.floor(width / 3));
  const end = Math.min(text.length, start + width);
  return `${start > 0 ? "…" : ""}${text.slice(start, end)}${end < text.length ? "…" : ""}`;
}

/**
 * Substring search that works for Chinese (no tokenizer needed): every term
 * must occur somewhere in the page; matches in the title count most, then id,
 * tags, summary, headings, body.
 */
export function search(pages: Page[], query: string, limit = 24): Hit[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return [];
  const hits: Hit[] = [];
  for (const doc of index(pages)) {
    let score = 0;
    let snippet = "";
    let anchor: string | null = null;
    let all = true;
    for (const term of terms) {
      let s = 0;
      if (doc.title.includes(term)) s += doc.title === term ? 120 : doc.title.startsWith(term) ? 90 : 70;
      if (doc.id.includes(term)) s += 50;
      if (doc.tags.includes(term)) s += 30;
      if (doc.summary.includes(term)) s += 20;
      const h = doc.headings.find((x) => x.text.includes(term));
      if (h) {
        s += 15;
        anchor ??= h.slug;
      }
      let bodyHits = 0;
      for (const line of doc.lines) {
        const at = line.text.toLowerCase().indexOf(term);
        if (at >= 0) {
          bodyHits++;
          if (!snippet) {
            snippet = clip(line.text, at);
            anchor ??= line.anchor;
          }
        }
      }
      s += Math.min(bodyHits, 10) * 2;
      if (s === 0) {
        all = false;
        break;
      }
      score += s;
    }
    if (!all) continue;
    hits.push({ page: doc.page, score, snippet: snippet || doc.page.summary, anchor });
  }
  hits.sort((a, b) => b.score - a.score || a.page.title.localeCompare(b.page.title, "zh-CN"));
  return hits.slice(0, limit);
}
