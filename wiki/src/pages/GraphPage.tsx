import { useEffect, useMemo, useRef, useState } from "react";
import { EDGE_KINDS, KINDS, type EdgeKind, type Kind, type Page } from "../core/wiki.js";
import { GraphView, KindGlyph, type GraphHandle } from "../components/GraphView";
import { KindTag } from "../components/KindTag";
import { Window } from "../components/Window";
import { sortedPages, wiki } from "../lib/data";
import { KIND_VAR } from "../lib/kinds";
import { graphHref, navigate, pageHref, replaceHash } from "../lib/router";
import { search } from "../lib/search";

/** A short line drawn the way the graph draws that edge kind. */
function EdgeSample({ kind }: { kind: EdgeKind }) {
  return (
    <svg width="26" height="10" aria-hidden="true">
      <line className="g-edge" data-kind={kind} x1="1" y1="5" x2="25" y2="5" style={{ strokeOpacity: 0.85 }} />
    </svg>
  );
}

function SelectedCard({ page, onSelect, onCenter }: { page: Page; onSelect: (id: string) => void; onCenter: () => void }) {
  const groups = useMemo(() => {
    const out = wiki.outgoing.get(page.id) ?? [];
    const inc = wiki.incoming.get(page.id) ?? [];
    const rows: { label: string; ids: string[] }[] = [
      { label: "所属", ids: out.filter((e) => e.kind === "parent").map((e) => e.target) },
      { label: "子页面", ids: inc.filter((e) => e.kind === "parent").map((e) => e.source) },
      { label: "依赖", ids: out.filter((e) => e.kind === "depends").map((e) => e.target) },
      { label: "被依赖", ids: inc.filter((e) => e.kind === "depends").map((e) => e.source) },
      { label: "链接到", ids: out.filter((e) => e.kind === "link").map((e) => e.target) },
      { label: "被链接", ids: inc.filter((e) => e.kind === "link").map((e) => e.source) },
      {
        label: "相关",
        ids: [...out.filter((e) => e.kind === "related").map((e) => e.target), ...inc.filter((e) => e.kind === "related").map((e) => e.source)],
      },
    ];
    return rows.filter((r) => r.ids.length > 0);
  }, [page]);

  return (
    <Window
      className="graph-card"
      icon="#"
      iconColor={KIND_VAR[page.kind]}
      title={page.title}
      tools={
        <button type="button" className="btn btn--small btn--square" onClick={() => onSelect("")} aria-label="关闭">
          ✕
        </button>
      }
    >
      <div className="graph-card__body">
        <div className="graph-card__meta">
          <KindTag kind={page.kind} />
          <span className="tag tag--mono">{page.id}</span>
        </div>
        <p>{page.summary}</p>
        <div className="graph-card__actions">
          <a className="btn btn--pink btn--small" href={pageHref(page.id)}>
            打开页面 →
          </a>
          <button type="button" className="btn btn--small" onClick={onCenter}>
            居中
          </button>
        </div>
        <dl className="graph-card__rels">
          {groups.map((g) => (
            <div key={g.label}>
              <dt>
                {g.label} <span className="mono muted">{new Set(g.ids).size}</span>
              </dt>
              <dd>
                {[...new Set(g.ids)].map((id) => {
                  const p = wiki.pages.get(id);
                  if (!p) return null;
                  return (
                    <button key={id} type="button" className="chip" data-kind={p.kind} onClick={() => onSelect(id)} title={p.title}>
                      {id}
                    </button>
                  );
                })}
              </dd>
            </div>
          ))}
        </dl>
      </div>
    </Window>
  );
}

/** Every page as a table row: the graph's text equivalent. */
function GraphTable({ pages }: { pages: Page[] }) {
  const rows = useMemo(
    () =>
      pages
        .map((p) => ({
          page: p,
          out: (wiki.outgoing.get(p.id) ?? []).length,
          inc: (wiki.incoming.get(p.id) ?? []).length,
        }))
        .sort((a, b) => b.out + b.inc - (a.out + a.inc) || a.page.title.localeCompare(b.page.title, "zh-CN")),
    [pages],
  );
  return (
    <div className="graph-table prose">
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>页面</th>
              <th>类型</th>
              <th>所属</th>
              <th style={{ textAlign: "right" }}>出边</th>
              <th style={{ textAlign: "right" }}>入边</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(({ page, out, inc }) => (
              <tr key={page.id}>
                <td>
                  <a href={pageHref(page.id)}>{page.title}</a>
                  <div className="mono muted" style={{ fontSize: 11.5 }}>
                    {page.id}
                  </div>
                </td>
                <td>
                  <KindTag kind={page.kind} />
                </td>
                <td className="mono">{page.parent ?? ""}</td>
                <td className="mono" style={{ textAlign: "right" }}>
                  {out}
                </td>
                <td className="mono" style={{ textAlign: "right" }}>
                  {inc}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export function GraphPage({ focus }: { focus: string | null }) {
  const allPages = useMemo(() => sortedPages(wiki.pages.values()), []);
  const [kinds, setKinds] = useState<Set<Kind>>(() => new Set(KINDS.map((k) => k.id)));
  const [edgeKinds, setEdgeKinds] = useState<Set<EdgeKind>>(() => new Set(EDGE_KINDS.map((k) => k.id)));
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(focus && wiki.pages.has(focus) ? focus : null);
  const [view, setView] = useState<"graph" | "table">("graph");
  const graphRef = useRef<GraphHandle>(null);

  useEffect(() => {
    document.title = "知识图谱 · phasor 知识库";
  }, []);

  useEffect(() => {
    if (focus && wiki.pages.has(focus)) {
      setSelected(focus);
      const t = window.setTimeout(() => graphRef.current?.centerOn(focus), 80);
      return () => window.clearTimeout(t);
    }
  }, [focus]);

  const pages = useMemo(() => allPages.filter((p) => kinds.has(p.kind)), [allPages, kinds]);
  const edges = useMemo(() => {
    const ids = new Set(pages.map((p) => p.id));
    return wiki.edges.filter((e) => edgeKinds.has(e.kind) && ids.has(e.source) && ids.has(e.target));
  }, [pages, edgeKinds]);
  const matches = useMemo(() => (query.trim() ? new Set(search(pages, query, 500).map((h) => h.page.id)) : undefined), [pages, query]);

  const counts = useMemo(() => {
    const c: Record<string, number> = {};
    for (const p of allPages) c[p.kind] = (c[p.kind] ?? 0) + 1;
    for (const e of wiki.edges) c[`e:${e.kind}`] = (c[`e:${e.kind}`] ?? 0) + 1;
    return c;
  }, [allPages]);

  const select = (id: string) => {
    const next = id && wiki.pages.has(id) ? id : null;
    setSelected(next);
    replaceHash(graphHref(next));
  };

  const toggle = <T,>(set: Set<T>, key: T): Set<T> => {
    const next = new Set(set);
    if (next.has(key)) {
      if (next.size > 1) next.delete(key);
    } else next.add(key);
    return next;
  };

  const selectedPage = selected ? wiki.pages.get(selected) : undefined;
  const firstMatch = matches && [...matches][0];

  return (
    <div className="graph-page">
      <Window
        icon="◎"
        iconColor="var(--cyan)"
        title="知识图谱"
        subtitle={`${pages.length} 页 · ${edges.length} 条关系`}
        tools={
          <span className="segmented" role="group" aria-label="视图">
            <button type="button" aria-pressed={view === "graph"} onClick={() => setView("graph")}>
              图
            </button>
            <button type="button" aria-pressed={view === "table"} onClick={() => setView("table")}>
              表
            </button>
          </span>
        }
      >
        <div className="graph-toolbar" role="toolbar" aria-label="图谱筛选">
          <span className="graph-toolbar__label mono">页面</span>
          {KINDS.map((k) => (
            <button
              key={k.id}
              type="button"
              className="btn btn--small"
              aria-pressed={kinds.has(k.id)}
              onClick={() => setKinds((s) => toggle(s, k.id))}
              title={`显示或隐藏${k.label}页面`}
            >
              <KindGlyph kind={k.id} />
              {k.label}
              <span className="mono">{counts[k.id] ?? 0}</span>
            </button>
          ))}
          <span className="sep" aria-hidden="true" />
          <span className="graph-toolbar__label mono">关系</span>
          {EDGE_KINDS.map((k) => (
            <button
              key={k.id}
              type="button"
              className="btn btn--small"
              aria-pressed={edgeKinds.has(k.id)}
              onClick={() => setEdgeKinds((s) => toggle(s, k.id))}
              title={`显示或隐藏「${k.label}」关系`}
            >
              <EdgeSample kind={k.id} />
              {k.label}
              <span className="mono">{counts[`e:${k.id}`] ?? 0}</span>
            </button>
          ))}
          <span className="sep" aria-hidden="true" />
          <input
            type="search"
            value={query}
            placeholder="高亮匹配的页面…"
            aria-label="在图谱中查找"
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && firstMatch) {
                select(firstMatch);
                graphRef.current?.centerOn(firstMatch);
              }
            }}
          />
          {matches ? <span className="mono muted">{matches.size} 个匹配</span> : null}
        </div>
        {view === "graph" ? (
          <div className="graph-wrap">
            <GraphView
              ref={graphRef}
              pages={pages}
              edges={edges}
              focus={selected}
              matches={matches}
              onSelect={select}
              onOpen={(id) => navigate(pageHref(id))}
              ariaLabel={`知识图谱：${pages.length} 页，${edges.length} 条关系。按 Tab 逐个聚焦节点，Enter 打开页面，空格选中。`}
            />
            {selectedPage ? (
              <SelectedCard page={selectedPage} onSelect={select} onCenter={() => graphRef.current?.centerOn(selectedPage.id)} />
            ) : null}
            <div className="graph-hint" aria-hidden="true">
              单击选中 · 双击打开 · 拖动节点 · 滚轮缩放 · 工具栏的按钮兼作图例
            </div>
            <div className="graph-zoom">
              <button type="button" className="btn btn--small btn--square" onClick={() => graphRef.current?.zoomBy(1.3)} aria-label="放大">
                +
              </button>
              <button type="button" className="btn btn--small btn--square" onClick={() => graphRef.current?.zoomBy(1 / 1.3)} aria-label="缩小">
                −
              </button>
              <button type="button" className="btn btn--small btn--square" onClick={() => graphRef.current?.fit()} aria-label="适应窗口" title="适应窗口">
                ⤢
              </button>
              <button type="button" className="btn btn--small btn--square" onClick={() => graphRef.current?.reheat()} aria-label="重新布局" title="重新布局">
                ↻
              </button>
            </div>
          </div>
        ) : (
          <GraphTable pages={pages} />
        )}
      </Window>
    </div>
  );
}
