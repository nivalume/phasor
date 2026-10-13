import { useMemo } from "react";
import type { Page } from "../core/wiki.js";
import { sortedPages, wiki } from "../lib/data";
import { KIND_LABEL } from "../lib/kinds";
import { pageHref } from "../lib/router";
import { PixelAvatar } from "./PixelAvatar";

/** Markdown line → readable text for a quote. */
function plain(line: string): string {
  return line
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/^\s*([-*+]|\d+\.|>|#+|\|)\s*(\[[ xX]\]\s*)?/, "")
    .replace(/[`*|]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

interface Mention {
  page: Page;
  quote: string;
}

/** Pages whose body links here, each with the line that holds the link (like a chat message quoting it). */
export function useMentions(page: Page): Mention[] {
  return useMemo(() => {
    const sources = (wiki.incoming.get(page.id) ?? []).filter((e) => e.kind === "link").map((e) => wiki.pages.get(e.source));
    const out: Mention[] = [];
    for (const src of sortedPages(sources.filter((p): p is Page => Boolean(p)))) {
      const link = src.links.find((l) => l.target.type === "page" && l.target.id === page.id && !l.image);
      const raw = link ? src.body.split("\n")[link.line - src.bodyLine] ?? "" : "";
      out.push({ page: src, quote: plain(raw) || src.summary });
    }
    return out;
  }, [page]);
}

export function Backlinks({ page }: { page: Page }) {
  const mentions = useMentions(page);
  if (mentions.length === 0) return <p className="muted" style={{ padding: "10px 14px", margin: 0 }}>还没有页面链接到这里。</p>;
  return (
    <div className="chat">
      {mentions.map(({ page: src, quote }) => (
        <a key={src.id} className="msg" href={pageHref(src.id)}>
          <PixelAvatar kind={src.kind} />
          <div style={{ minWidth: 0 }}>
            <div className="msg__head">
              <span className="msg__name">{src.title}</span>
              <span className="msg__time">{KIND_LABEL[src.kind]}</span>
            </div>
            <div className="msg__text">{quote}</div>
          </div>
        </a>
      ))}
    </div>
  );
}
