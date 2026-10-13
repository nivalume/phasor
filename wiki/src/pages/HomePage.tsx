import { useEffect, useMemo, useState } from "react";
import { KINDS, type Kind, type Page } from "../core/wiki.js";
import { GraphView } from "../components/GraphView";
import { KindGlyph } from "../components/GraphView";
import { Markdown } from "../components/Markdown";
import { PixelAvatar } from "../components/PixelAvatar";
import { Window } from "../components/Window";
import { CODE_REF } from "../config";
import { getPage, sortedPages, wiki } from "../lib/data";
import { KIND_LABEL, KIND_VAR } from "../lib/kinds";
import { graphHref, navigate, pageHref } from "../lib/router";

/** Pages shown as the chat feed in the hero window, in this order, when they exist. */
const FEED = ["reading-roadmap", "system-map", "conventions", "barq-exec", "bar-loop", "pratt-parser", "research-protocol"];

const STICKERS: { kind: Kind; style: React.CSSProperties }[] = [
  { kind: "crate", style: { top: -30, left: -26, transform: "rotate(-8deg)" } },
  { kind: "algorithm", style: { top: 70, right: -30, transform: "rotate(7deg)" } },
  { kind: "concept", style: { bottom: 58, left: -34, transform: "rotate(5deg)" } },
  { kind: "tooling", style: { bottom: -28, right: 48, transform: "rotate(-6deg)" } },
];

function Feed() {
  const pages = FEED.map((id) => wiki.pages.get(id)).filter((p): p is Page => Boolean(p));
  return (
    <div className="chat hero-feed">
      {pages.map((p, i) => (
        <a key={p.id} className="msg" href={pageHref(p.id)}>
          <PixelAvatar kind={p.kind} online={i % 3 !== 2} />
          <div style={{ minWidth: 0 }}>
            <div className="msg__head">
              <span className="msg__name">{p.title}</span>
              <span className="msg__time">
                {KIND_LABEL[p.kind]} · #{p.id}
              </span>
            </div>
            <div className="msg__text">{p.summary}</div>
          </div>
        </a>
      ))}
    </div>
  );
}

function Board() {
  const pages = useMemo(() => sortedPages(wiki.pages.values()).filter((p) => p.id !== "index"), []);
  return (
    <div className="home-board">
      {KINDS.map((k) => {
        const list = pages.filter((p) => p.kind === k.id);
        return (
          <section key={k.id} className="board__col" aria-label={k.label}>
            <div className="board__head">
              <KindGlyph kind={k.id} />
              <strong>{k.label}</strong>
              <span className="count" style={{ marginLeft: "auto" }}>
                {list.length}
              </span>
            </div>
            <div className="board__cards">
              {list.map((p) => (
                <a key={p.id} className="kcard" href={pageHref(p.id)} title={p.summary}>
                  <div className="kcard__id">{p.id}</div>
                  <div className="kcard__title">{p.title}</div>
                  {p.parent ? (
                    <div className="kcard__foot">
                      <span className="tag tag--mono">↑ {p.parent}</span>
                    </div>
                  ) : null}
                </a>
              ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}

export function HomePage() {
  const [tab, setTab] = useState<"feed" | "graph">("feed");
  const index = getPage("index");
  const pages = useMemo(() => [...wiki.pages.values()], []);
  const counts = useMemo(() => {
    const c: Record<string, number> = {};
    for (const p of pages) c[p.kind] = (c[p.kind] ?? 0) + 1;
    return c;
  }, [pages]);
  // The index page without its H1: the hero already carries the title.
  const indexBody = useMemo(() => (index ? { ...index, body: index.body.replace(/^#\s.*\n+/, "") } : null), [index]);

  useEffect(() => {
    document.title = "barq 知识库";
  }, []);

  return (
    <>
      <section className="hero">
        <div>
          <span className="tag tag--mono hero__kicker">● barq / 源码阅读</span>
          <h1>
            barq 源码
            <br />
            <span className="marker">知识库</span>
          </h1>
          <p>
            13 个 crate 的阅读笔记：定位、文件阅读顺序、类图、核心算法、调用链和阅读陷阱。共 {pages.length} 页，每页讲一件事；页面之间的{" "}
            {wiki.edges.length} 条关系可以在知识图谱里查看。
          </p>
          <div className="hero__cta">
            <a className="btn btn--pink" href={pageHref("reading-roadmap")}>
              阅读路线 →
            </a>
            <a className="btn btn--yellow" href={graphHref()}>
              ◎ 知识图谱
            </a>
            <a className="btn" href={pageHref("system-map")}>
              系统全景
            </a>
          </div>
        </div>
        <div className="hero__stage">
          {STICKERS.map((s) => (
            <div key={s.kind} className="sticker" style={{ ...s.style, background: KIND_VAR[s.kind] }} aria-hidden="true">
              <PixelAvatar kind={s.kind} size={50} />
            </div>
          ))}
          <Window dotted icon="#" title="barq-wiki" subtitle={`${pages.length} 页 · ${wiki.edges.length} 条关系`}>
            <div className="tabs" role="tablist" aria-label="首页窗口">
              <button type="button" role="tab" aria-selected={tab === "feed"} onClick={() => setTab("feed")}>
                # 从这里读起
              </button>
              <button type="button" role="tab" aria-selected={tab === "graph"} onClick={() => setTab("graph")}>
                ◎ 图谱
              </button>
            </div>
            {tab === "feed" ? (
              <Feed />
            ) : (
              <div className="hero-graph">
                <GraphView
                  pages={pages}
                  edges={wiki.edges}
                  onSelect={(id) => id && navigate(graphHref(id))}
                  onOpen={(id) => navigate(pageHref(id))}
                  ariaLabel={`知识图谱：${pages.length} 页，${wiki.edges.length} 条关系`}
                />
              </div>
            )}
          </Window>
        </div>
      </section>

      <section className="band" aria-label="这份知识库回答的问题">
        <blockquote>按什么顺序读、每段代码的核心算法是什么、看不懂时去哪里补课。</blockquote>
        <div className="stats">
          <span className="tag">{pages.length} 页</span>
          <span className="tag">{wiki.edges.length} 条关系</span>
          {KINDS.map((k) => (
            <span key={k.id} className="tag" data-kind={k.id}>
              {k.label} {counts[k.id] ?? 0}
            </span>
          ))}
        </div>
      </section>

      <section className="section">
        <h2>全部页面</h2>
        <Window icon="▦" iconColor="var(--lime)" title="看板" subtitle="按页面类型分列" dotted>
          <Board />
        </Window>
      </section>

      {indexBody ? (
        <section className="section section--narrow">
          <h2>知识库怎么组织</h2>
          <Window icon="#" title={<span className="mono">index</span>} subtitle="wiki/content/index.md">
            <div className="page-body">
              <Markdown page={indexBody} />
            </div>
          </Window>
          <p className="muted mono home-note">代码链接指向提交 {CODE_REF.slice(0, 7)}，页面里的行号都按这个版本核对过。</p>
        </section>
      ) : null}
    </>
  );
}
