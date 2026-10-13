import { Fragment, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { sortedPages, wiki } from "../lib/data";
import { KIND_LABEL } from "../lib/kinds";
import { navigate, pageHref } from "../lib/router";
import { search, type Hit } from "../lib/search";
import { PixelAvatar } from "./PixelAvatar";
import { Window } from "./Window";

/** Wrap every occurrence of the query terms in <mark>. */
function highlight(text: string, query: string): ReactNode {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return text;
  const lower = text.toLowerCase();
  const marks: [number, number][] = [];
  for (const t of terms) {
    let at = lower.indexOf(t);
    while (at >= 0) {
      marks.push([at, at + t.length]);
      at = lower.indexOf(t, at + t.length);
    }
  }
  if (marks.length === 0) return text;
  marks.sort((a, b) => a[0] - b[0]);
  const out: ReactNode[] = [];
  let pos = 0;
  marks.forEach(([s, e], i) => {
    if (s < pos) return;
    out.push(<Fragment key={`t${i}`}>{text.slice(pos, s)}</Fragment>);
    out.push(<mark key={`m${i}`}>{text.slice(s, e)}</mark>);
    pos = e;
  });
  out.push(<Fragment key="end">{text.slice(pos)}</Fragment>);
  return out;
}

const STARTERS = ["reading-roadmap", "system-map", "conventions", "algorithm-index", "glossary", "exercises"];

export function CommandPalette({ onClose }: { onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const hits: Hit[] = useMemo(() => {
    if (query.trim()) return search(sortedPages(wiki.pages.values()), query, 30);
    return STARTERS.map((id) => wiki.pages.get(id))
      .filter((p): p is NonNullable<typeof p> => Boolean(p))
      .map((page) => ({ page, score: 0, snippet: page.summary, anchor: null }));
  }, [query]);

  useEffect(() => setActive(0), [query]);
  useEffect(() => {
    inputRef.current?.focus();
    const previous = document.activeElement as HTMLElement | null;
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = overflow;
      previous?.focus?.();
    };
  }, []);
  useEffect(() => {
    listRef.current?.querySelector("[aria-selected='true']")?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const open = (hit: Hit | undefined) => {
    if (!hit) return;
    onClose();
    navigate(pageHref(hit.page.id, hit.anchor));
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, hits.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      open(hits[active]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "Tab") {
      // The dialog has one control; keep focus inside it.
      e.preventDefault();
    }
  };

  return (
    <div className="palette-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="palette" role="dialog" aria-modal="true" aria-label="搜索知识库" onKeyDown={onKey}>
        <Window
          icon="⌕"
          iconColor="var(--pink)"
          title={
            <input
              ref={inputRef}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="搜索页面、算法、函数名、文件…"
              aria-label="搜索"
              role="combobox"
              aria-expanded="true"
              aria-controls="palette-results"
              aria-activedescendant={hits[active] ? `hit-${hits[active].page.id}` : undefined}
              autoComplete="off"
              spellCheck={false}
            />
          }
          tools={
            <span className="kbd" aria-hidden="true">
              esc
            </span>
          }
          className="palette__window"
        >
          <div className="palette__label mono muted">{query.trim() ? `${hits.length} 条结果` : "从这里开始"}</div>
          <div className="palette__results chat" id="palette-results" role="listbox" ref={listRef}>
            {hits.length === 0 ? (
              <div className="palette__empty">没有匹配的页面。换个关键词，或者去知识图谱里找。</div>
            ) : (
              hits.map((hit, i) => (
                <a
                  key={hit.page.id}
                  id={`hit-${hit.page.id}`}
                  className="msg"
                  role="option"
                  aria-selected={i === active}
                  href={pageHref(hit.page.id, hit.anchor)}
                  onMouseMove={() => setActive(i)}
                  onClick={(e) => {
                    e.preventDefault();
                    open(hit);
                  }}
                >
                  <PixelAvatar kind={hit.page.kind} />
                  <div style={{ minWidth: 0 }}>
                    <div className="msg__head">
                      <span className="msg__name">{highlight(hit.page.title, query)}</span>
                      <span className="msg__time">
                        {KIND_LABEL[hit.page.kind]} · {hit.page.id}
                        {hit.anchor ? ` # ${hit.anchor}` : ""}
                      </span>
                    </div>
                    <div className="msg__text">{highlight(hit.snippet, query)}</div>
                  </div>
                </a>
              ))
            )}
          </div>
          <div className="palette__foot mono muted">
            <span>
              <span className="kbd">↑</span> <span className="kbd">↓</span> 选择
            </span>
            <span>
              <span className="kbd">enter</span> 打开
            </span>
            <span>
              <span className="kbd">esc</span> 关闭
            </span>
          </div>
        </Window>
      </div>
    </div>
  );
}
