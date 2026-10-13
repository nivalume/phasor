import { useEffect, useMemo, useState } from "react";
import type { Kind, Page } from "../core/wiki.js";
import { childrenOf, sortedPages, wiki } from "../lib/data";
import { KIND_LABEL, KIND_VAR } from "../lib/kinds";
import { graphHref, pageHref } from "../lib/router";
import { Window } from "./Window";

/** Top-level groups of the channel list. Concepts and algorithms sit under their crate or tooling page. */
const GROUPS: { kind: Kind; label: string }[] = [
  { kind: "guide", label: "指南" },
  { kind: "crate", label: "Crate" },
  { kind: "reference", label: "参考" },
  { kind: "tooling", label: "工具链" },
];

function Channel({ page, current, child = false }: { page: Page; current: string | null; child?: boolean }) {
  return (
    <a
      className={`channel${child ? " channel--child" : ""}`}
      href={pageHref(page.id)}
      aria-current={page.id === current ? "page" : undefined}
      title={page.summary}
    >
      <span className="channel__hash" aria-hidden="true">
        {child ? (page.kind === "algorithm" ? "◆" : "●") : "#"}
      </span>
      <span>{page.title}</span>
    </a>
  );
}

export function Sidebar({ current, onSearch }: { current: string | null; onSearch: () => void }) {
  const pages = useMemo(() => sortedPages(wiki.pages.values()), []);
  const currentPage = current ? wiki.pages.get(current) : undefined;
  const activeParent = currentPage?.parent ?? (currentPage && childrenOf(currentPage.id).length > 0 ? currentPage.id : null);

  const [open, setOpen] = useState<Set<string>>(() => new Set(activeParent ? [activeParent] : []));
  const [collapsed, setCollapsed] = useState<Set<Kind>>(new Set());

  // Opening a page expands its crate in the list.
  useEffect(() => {
    if (activeParent) setOpen((s) => (s.has(activeParent) ? s : new Set(s).add(activeParent)));
  }, [activeParent]);

  // Keep the current channel in view inside the list (without scrolling the page itself).
  useEffect(() => {
    const list = document.querySelector<HTMLElement>(".channels__list");
    const el = list?.querySelector<HTMLElement>(".channel[aria-current='page']");
    if (!list || !el) return;
    const delta = el.getBoundingClientRect().top - list.getBoundingClientRect().top;
    if (delta < 0 || delta > list.clientHeight - 40) list.scrollTop += delta - list.clientHeight / 3;
  }, [current]);

  const toggle = <T,>(set: Set<T>, key: T) => {
    const next = new Set(set);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    return next;
  };

  return (
    <Window as="nav" className="channels" title="频道" subtitle={`${pages.length} 页`} labelledBy="channels-title">
      <button type="button" className="channels__search" onClick={onSearch}>
        <span aria-hidden="true">⌕</span>
        <span>搜索页面、算法、函数…</span>
        <span className="kbd" style={{ marginLeft: "auto" }}>
          /
        </span>
      </button>
      <div className="channels__list">
        <a className="channel" href="#/" aria-current={current === null ? "page" : undefined}>
          <span className="channel__hash" aria-hidden="true">
            ~
          </span>
          <span>首页</span>
        </a>
        <a className="channel" href={graphHref()}>
          <span className="channel__hash" aria-hidden="true">
            ◎
          </span>
          <span>知识图谱</span>
        </a>
        {GROUPS.map(({ kind, label }) => {
          const top = pages.filter((p) => p.kind === kind && p.id !== "index");
          if (top.length === 0) return null;
          const isCollapsed = collapsed.has(kind);
          const total = top.reduce((n, p) => n + 1 + childrenOf(p.id).length, 0);
          return (
            <div className="channels__group" key={kind}>
              <button
                type="button"
                className="channels__heading"
                aria-expanded={!isCollapsed}
                onClick={() => setCollapsed((s) => toggle(s, kind))}
              >
                <span className="swatch" style={{ background: KIND_VAR[kind] }} aria-hidden="true" />
                <span>
                  {isCollapsed ? "▸" : "▾"} {label}
                </span>
                <span className="count">{total}</span>
              </button>
              {isCollapsed
                ? null
                : top.map((page) => {
                    const kids = childrenOf(page.id);
                    const expanded = open.has(page.id);
                    return (
                      <div key={page.id}>
                        <div className="channel-row">
                          <Channel page={page} current={current} />
                          {kids.length > 0 ? (
                            <button
                              type="button"
                              className="channel__caret"
                              aria-expanded={expanded}
                              aria-label={`${expanded ? "收起" : "展开"} ${page.title} 的 ${kids.length} 个子页面`}
                              onClick={() => setOpen((s) => toggle(s, page.id))}
                            >
                              <span aria-hidden="true">{expanded ? "−" : "+"}</span>
                              <span className="mono">{kids.length}</span>
                            </button>
                          ) : null}
                        </div>
                        {expanded ? kids.map((k) => <Channel key={k.id} page={k} current={current} child />) : null}
                      </div>
                    );
                  })}
            </div>
          );
        })}
        <p className="channels__legend muted">
          <span>● {KIND_LABEL.concept}</span>
          <span>◆ {KIND_LABEL.algorithm}</span>
        </p>
      </div>
    </Window>
  );
}
