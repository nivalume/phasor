import { useEffect, useMemo, useState } from "react";
import { parseSourceRef, type Page } from "../core/wiki.js";
import { Backlinks } from "../components/Backlinks";
import { KindTag } from "../components/KindTag";
import { LocalGraph } from "../components/LocalGraph";
import { Markdown } from "../components/Markdown";
import { Toc } from "../components/Toc";
import { Window } from "../components/Window";
import { childrenOf, sortedPages, wiki } from "../lib/data";
import { editUrl, githubUrl } from "../lib/github";
import { KIND_LABEL, KIND_VAR } from "../lib/kinds";
import { graphHref, pageHref } from "../lib/router";

type Tab = "body" | "relations" | "sources";


function Card({ page, note }: { page: Page; note?: string }) {
  return (
    <a className="kcard" href={pageHref(page.id)} title={page.summary}>
      <div className="kcard__id">{page.id}</div>
      <div className="kcard__title">{page.title}</div>
      <div className="kcard__foot">
        {note ? <span className="tag tag--mono">{note}</span> : null}
        <KindTag kind={page.kind} />
      </div>
    </a>
  );
}

/** Every relation of the page, one column per relation type. */
function Relations({ page }: { page: Page }) {
  const columns = useMemo(() => {
    const get = (id: string) => wiki.pages.get(id);
    const list = (ids: Iterable<string>) => sortedPages([...new Set(ids)].map(get).filter((p): p is Page => Boolean(p)));
    const out = wiki.outgoing.get(page.id) ?? [];
    const inc = wiki.incoming.get(page.id) ?? [];
    const by = (edges: typeof out, kind: string, end: "source" | "target") => edges.filter((e) => e.kind === kind).map((e) => e[end]);
    return [
      { title: "所属", hint: "parent", pages: list(by(out, "parent", "target")) },
      { title: "子页面", hint: "children", pages: childrenOf(page.id) },
      { title: "依赖", hint: "depends", pages: list(by(out, "depends", "target")) },
      { title: "被依赖", hint: "depended on by", pages: list(by(inc, "depends", "source")) },
      { title: "链接到", hint: "links to", pages: list(by(out, "link", "target")) },
      { title: "被链接", hint: "linked from", pages: list(by(inc, "link", "source")) },
      { title: "相关", hint: "related", pages: list([...by(out, "related", "target"), ...by(inc, "related", "source")]) },
    ].filter((c) => c.pages.length > 0);
  }, [page]);

  return (
    <div className="board">
      {columns.map((col) => (
        <section className="board__col" key={col.title} aria-label={col.title}>
          <div className="board__head">
            <strong>{col.title}</strong>
            <span className="mono muted">{col.hint}</span>
            <span className="count" style={{ marginLeft: "auto" }}>
              {col.pages.length}
            </span>
          </div>
          <div className="board__cards">
            {col.pages.map((p) => (
              <Card key={p.id} page={p} />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

function Sources({ page }: { page: Page }) {
  if (page.sources.length === 0) {
    return <p className="muted" style={{ padding: "20px", margin: 0 }}>这一页没有列出源码位置。</p>;
  }
  return (
    <div className="page-body prose">
      <p>这一页讲的代码。行号对应知识库核对时的提交，链接打开的就是那个版本。</p>
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>文件</th>
              <th>行</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {page.sources.map((ref) => {
              const { path, start, end } = parseSourceRef(ref);
              return (
                <tr key={ref}>
                  <td>
                    <code>{path}</code>
                  </td>
                  <td className="mono">{start == null ? "全文" : end == null ? start : `${start}–${end}`}</td>
                  <td>
                    <a href={githubUrl(ref)} target="_blank" rel="noreferrer" data-external="true">
                      GitHub
                    </a>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export function WikiPage({ page, anchor }: { page: Page; anchor: string | null }) {
  const [tab, setTab] = useState<Tab>("body");
  const ordered = useMemo(() => sortedPages(wiki.pages.values()), []);
  const at = ordered.findIndex((p) => p.id === page.id);
  const prev = at > 0 ? ordered[at - 1] : null;
  const next = at >= 0 && at < ordered.length - 1 ? ordered[at + 1] : null;
  const parent = page.parent ? wiki.pages.get(page.parent) : null;
  const degree = (wiki.outgoing.get(page.id)?.length ?? 0) + (wiki.incoming.get(page.id)?.length ?? 0);

  useEffect(() => setTab("body"), [page.id]);
  useEffect(() => {
    document.title = `${page.title} · phasor 知识库`;
  }, [page]);

  // Scroll to the anchor (or the top) after each navigation; again once diagrams above it have drawn.
  useEffect(() => {
    if (tab !== "body") return;
    const go = () => {
      const el = anchor ? document.getElementById(anchor) : null;
      if (el) el.scrollIntoView({ block: "start" });
      else if (!anchor) window.scrollTo({ top: 0 });
    };
    go();
    if (!anchor) return;
    const t = window.setTimeout(go, 600);
    return () => window.clearTimeout(t);
  }, [page.id, anchor, tab]);

  const tabs: { id: Tab; label: string; count?: number }[] = [
    { id: "body", label: "正文" },
    { id: "relations", label: "关系", count: degree },
    { id: "sources", label: "源码", count: page.sources.length },
  ];

  return (
    <>
      <Window
        as="article"
        className="page-window"
        icon="#"
        iconColor={KIND_VAR[page.kind]}
        title={<span className="mono">{page.id}</span>}
        subtitle={page.path.replace(/^wiki\/content\//, "")}
        tools={
          <>
            <a className="btn btn--small hide-narrow" href={graphHref(page.id)}>
              ◎ 图谱
            </a>
            <a className="btn btn--small hide-narrow" href={editUrl(page.path)} target="_blank" rel="noreferrer">
              ✎ 编辑
            </a>
          </>
        }
      >
        <div className="tabs" role="tablist" aria-label="页面视图">
          {tabs.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={tab === t.id}
              aria-controls={`panel-${t.id}`}
              onClick={() => setTab(t.id)}
            >
              {t.label}
              {t.count != null ? <span className="mono muted">{t.count}</span> : null}
            </button>
          ))}
        </div>
        <div className="page-meta">
          <KindTag kind={page.kind} />
          {parent ? (
            <>
              <span className="label">所属</span>
              <a className="tag" data-kind={parent.kind} href={pageHref(parent.id)}>
                {parent.title.split("：")[0]}
              </a>
            </>
          ) : null}
          {page.depends.length > 0 ? (
            <>
              <span className="label">依赖</span>
              {page.depends.map((d) => (
                <a key={d} className="tag tag--mono" href={pageHref(d)}>
                  {d}
                </a>
              ))}
            </>
          ) : null}
          {page.tags.length > 0 ? (
            <>
              <span className="label">标签</span>
              {page.tags.map((t) => (
                <span key={t} className="tag tag--mono">
                  {t}
                </span>
              ))}
            </>
          ) : null}
        </div>
        <div role="tabpanel" id={`panel-${tab}`}>
          {tab === "body" ? (
            <>
              <div className="page-body">
                <Markdown page={page} />
              </div>
              <nav className="pager" aria-label="上一页与下一页">
                {prev ? (
                  <a href={pageHref(prev.id)}>
                    <div className="dir">← 上一页 · {KIND_LABEL[prev.kind]}</div>
                    <div className="ttl">{prev.title}</div>
                  </a>
                ) : (
                  <span />
                )}
                {next ? (
                  <a href={pageHref(next.id)}>
                    <div className="dir">下一页 · {KIND_LABEL[next.kind]} →</div>
                    <div className="ttl">{next.title}</div>
                  </a>
                ) : null}
              </nav>
            </>
          ) : tab === "relations" ? (
            <Relations page={page} />
          ) : (
            <Sources page={page} />
          )}
        </div>
      </Window>
    </>
  );
}

/** The right-hand column of a page: local graph, sections, backlinks. */
export function PageRail({ page }: { page: Page }) {
  const incomingLinks = (wiki.incoming.get(page.id) ?? []).filter((e) => e.kind === "link").length;
  return (
    <>
      <LocalGraph page={page} />
      <Window icon="≡" iconColor="var(--lime)" title="本页小节" subtitle={`${page.headings.filter((h) => h.depth === 2).length} 节`}>
        <Toc page={page} />
      </Window>
      <Window icon="↩" iconColor="var(--lavender)" title="谁提到了这里" subtitle={`${incomingLinks}`}>
        <Backlinks page={page} />
      </Window>
    </>
  );
}
