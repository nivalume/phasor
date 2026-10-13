import { useMemo, useState } from "react";
import { neighbours, type Page } from "../core/wiki.js";
import { wiki } from "../lib/data";
import { graphHref, navigate, pageHref } from "../lib/router";
import { GraphView } from "./GraphView";
import { Window } from "./Window";

/**
 * The current page and its neighbours, one or two hops out. Only edges that
 * step outward (hop n to hop n+1) are drawn, so the picture stays a fan rather
 * than a tangle of links among the neighbours. Clicking a node opens it.
 */
export function LocalGraph({ page }: { page: Page }) {
  const [depth, setDepth] = useState(1);
  const { pages, edges } = useMemo(() => {
    const hop = new Map<string, number>([[page.id, 0]]);
    let frontier = [page.id];
    for (let d = 1; d <= depth; d++) {
      const next: string[] = [];
      for (const id of frontier) {
        for (const n of neighbours(wiki, id)) {
          if (!hop.has(n)) {
            hop.set(n, d);
            next.push(n);
          }
        }
      }
      frontier = next;
    }
    return {
      pages: [...hop.keys()].map((id) => wiki.pages.get(id)).filter((p): p is Page => Boolean(p)),
      edges: wiki.edges.filter((e) => {
        const a = hop.get(e.source);
        const b = hop.get(e.target);
        return a != null && b != null && Math.abs(a - b) === 1;
      }),
    };
  }, [page, depth]);

  return (
    <Window
      icon="◎"
      iconColor="var(--cyan)"
      title="局部图谱"
      subtitle={`${pages.length - 1}`}
      tools={
        <span className="segmented" role="group" aria-label="跳数">
          {[1, 2].map((d) => (
            <button key={d} type="button" aria-pressed={depth === d} onClick={() => setDepth(d)}>
              {d}跳
            </button>
          ))}
        </span>
      }
    >
      <GraphView
        pages={pages}
        edges={edges}
        focus={page.id}
        mode="local"
        onOpen={(id) => navigate(pageHref(id))}
        ariaLabel={`${page.title} 的关系图：${pages.length - 1} 个相邻页面`}
      />
      <div className="window__foot">
        <a href={graphHref(page.id)}>在全局图谱中查看 →</a>
      </div>
    </Window>
  );
}
