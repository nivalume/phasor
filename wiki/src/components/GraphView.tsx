import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { drag as d3drag } from "d3-drag";
import {
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type Simulation,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from "d3-force";
import { select, type Selection } from "d3-selection";
import { symbol, symbolCircle, symbolCross, symbolDiamond, symbolSquare, symbolStar, symbolTriangle, type SymbolType } from "d3-shape";
import "d3-transition";
import { zoom as d3zoom, zoomIdentity, type ZoomBehavior } from "d3-zoom";
import type { Edge, Kind, Page } from "../core/wiki.js";
import { KIND_LABEL, KIND_VAR } from "../lib/kinds";

/** Shape per kind: the second channel, so kind never rests on colour alone. */
export const KIND_SYMBOL: Record<Kind, SymbolType> = {
  guide: symbolStar,
  crate: symbolSquare,
  concept: symbolCircle,
  algorithm: symbolDiamond,
  reference: symbolTriangle,
  tooling: symbolCross,
};

interface SimNode extends SimulationNodeDatum {
  id: string;
  kind: Kind;
  title: string;
  r: number;
  degree: number;
}

interface SimLink extends SimulationLinkDatum<SimNode> {
  kind: Edge["kind"];
}

export interface GraphHandle {
  fit: () => void;
  reheat: () => void;
  centerOn: (id: string) => void;
  zoomBy: (factor: number) => void;
}

export interface GraphViewProps {
  pages: Page[];
  edges: Edge[];
  /** Highlighted page (the current page, or the selected node). */
  focus?: string | null;
  /** Pages matching a search, outlined. */
  matches?: Set<string>;
  /** "local": small, every label shown, single click opens. */
  mode?: "global" | "local";
  onSelect?: (id: string) => void;
  onOpen?: (id: string) => void;
  ariaLabel: string;
}

const LINK_DISTANCE: Record<Edge["kind"], number> = { parent: 46, depends: 120, link: 95, related: 110 };
const LINK_STRENGTH: Record<Edge["kind"], number> = { parent: 0.9, depends: 0.25, link: 0.12, related: 0.08 };

function nodeRadius(kind: Kind, degree: number, local: boolean): number {
  const base = kind === "crate" ? 12 : kind === "guide" ? 11 : 6.5;
  const r = base + Math.min(Math.sqrt(degree) * 1.15, 9);
  return local ? r + 2 : r;
}

/** Graph label: a crate's id, otherwise the title up to its colon, kept short. */
function shortLabel(page: Page): string {
  if (page.kind === "crate") return page.id;
  const head = page.title.split("：")[0];
  return head.length <= 16 ? head : `${head.slice(0, 15)}…`;
}

/**
 * Where each page wants to sit in the global graph, so the picture reads like
 * the architecture: crates in columns by dependency depth (barq-core on the
 * left, the façades on the right), concepts and algorithms next to their crate,
 * guides along the top, reference and tooling pages along the bottom. Body
 * links and "related" edges are drawn but do not move nodes; with ~15 links
 * per page they would pull everything into one ball.
 */
function structuralTargets(pages: Page[]): Map<string, { x: number; y: number; band: "top" | "middle" | "bottom" }> {
  const byId = new Map(pages.map((p) => [p.id, p]));
  const depth = new Map<string, number>();
  const crateDepth = (id: string, seen = new Set<string>()): number => {
    const known = depth.get(id);
    if (known != null) return known;
    if (seen.has(id)) return 0;
    seen.add(id);
    const deps = (byId.get(id)?.depends ?? []).filter((d) => byId.has(d));
    const d = deps.length ? 1 + Math.max(...deps.map((x) => crateDepth(x, seen))) : 0;
    depth.set(id, d);
    return d;
  };
  const crates = pages.filter((p) => p.kind === "crate");
  crates.forEach((c) => crateDepth(c.id));
  const maxDepth = Math.max(1, ...depth.values());
  const COLUMN = 300;
  const width = maxDepth * COLUMN;
  const out = new Map<string, { x: number; y: number; band: "top" | "middle" | "bottom" }>();
  for (const c of crates) out.set(c.id, { x: (depth.get(c.id) ?? 0) * COLUMN, y: 0, band: "middle" });

  const spread = (list: Page[], y: number, band: "top" | "bottom") => {
    list.forEach((p, i) => out.set(p.id, { x: list.length === 1 ? width / 2 : (i / (list.length - 1)) * width, y, band }));
  };
  const loose = (kind: Kind) => pages.filter((p) => p.kind === kind && !(p.parent && byId.has(p.parent)));
  spread(loose("guide"), -360, "top");
  spread([...loose("reference"), ...loose("tooling")], 360, "bottom");

  for (const p of pages) {
    if (out.has(p.id)) continue;
    const parent = p.parent ? out.get(p.parent) : undefined;
    out.set(p.id, parent ? { ...parent } : { x: width / 2, y: 0, band: "middle" });
  }
  return out;
}

function symbolPath(kind: Kind, r: number): string {
  // d3 symbol size is an area; scale so every shape reads about as big as a circle of radius r.
  const area =
    kind === "concept" ? Math.PI * r * r : kind === "guide" ? 2.6 * r * r : kind === "reference" ? 3.9 * r * r : 3.4 * r * r;
  return symbol(KIND_SYMBOL[kind] ?? symbolCircle, area)() ?? "";
}

export const GraphView = forwardRef<GraphHandle, GraphViewProps>(function GraphView(
  { pages, edges, focus = null, matches, mode = "global", onSelect, onOpen, ariaLabel },
  ref,
) {
  const stageRef = useRef<HTMLDivElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const simRef = useRef<Simulation<SimNode, SimLink> | null>(null);
  const zoomRef = useRef<ZoomBehavior<SVGSVGElement, unknown> | null>(null);
  const nodesRef = useRef<SimNode[]>([]);
  const nodeSelRef = useRef<Selection<SVGGElement, SimNode, SVGGElement, unknown> | null>(null);
  const linkSelRef = useRef<Selection<SVGLineElement, SimLink, SVGGElement, unknown> | null>(null);
  const neighboursRef = useRef<Map<string, Set<string>>>(new Map());
  const [tip, setTip] = useState<{ id: string; x: number; y: number } | null>(null);
  const local = mode === "local";

  // Callbacks change identity every render; keep the latest in refs so the layout effect does not rerun.
  const onSelectRef = useRef(onSelect);
  const onOpenRef = useRef(onOpen);
  onSelectRef.current = onSelect;
  onOpenRef.current = onOpen;

  const byId = useMemo(() => new Map(pages.map((p) => [p.id, p])), [pages]);

  // Layout: rebuilt only when the set of pages or edges changes.
  useEffect(() => {
    const svgEl = svgRef.current;
    const stage = stageRef.current;
    if (!svgEl || !stage) return;
    const width = stage.clientWidth || 800;
    const height = stage.clientHeight || 500;

    const degree = new Map<string, number>();
    for (const e of edges) {
      degree.set(e.source, (degree.get(e.source) ?? 0) + 1);
      degree.set(e.target, (degree.get(e.target) ?? 0) + 1);
    }
    const previous = new Map(nodesRef.current.map((n) => [n.id, n]));
    const nodes: SimNode[] = pages.map((p) => {
      const old = previous.get(p.id);
      const d = degree.get(p.id) ?? 0;
      return {
        id: p.id,
        kind: p.kind,
        title: p.title,
        degree: d,
        r: nodeRadius(p.kind, d, local),
        x: old?.x,
        y: old?.y,
      };
    });
    nodesRef.current = nodes;
    const nodeById = new Map(nodes.map((n) => [n.id, n]));
    // Drawn edges point at node objects directly, so edges that take no part in the forces still have positions.
    const links: SimLink[] = edges
      .filter((e) => nodeById.has(e.source) && nodeById.has(e.target))
      .map((e) => ({ source: nodeById.get(e.source)!, target: nodeById.get(e.target)!, kind: e.kind }));

    const svg = select(svgEl);
    svg.selectAll("*").remove();
    const root = svg.append("g").attr("class", "g-root");
    const linkSel = root
      .append("g")
      .selectAll<SVGLineElement, SimLink>("line")
      .data(links)
      .join("line")
      .attr("class", "g-edge")
      .attr("data-kind", (d) => d.kind);

    const nodeSel = root
      .append("g")
      .selectAll<SVGGElement, SimNode>("g")
      .data(nodes, (d) => d.id)
      .join("g")
      .attr("class", "g-node")
      .attr("tabindex", 0)
      .attr("role", "link")
      .attr("aria-label", (d) => `${d.title}（${KIND_LABEL[d.kind] ?? d.kind}）`)
      .attr("data-kind", (d) => d.kind);
    nodeSel.append("circle").attr("class", "ring").attr("r", (d) => d.r + 6);
    nodeSel
      .append("path")
      .attr("class", "shadow")
      .attr("transform", "translate(2.5,2.5)")
      .attr("d", (d) => symbolPath(d.kind, d.r));
    nodeSel
      .append("path")
      .attr("class", "shape")
      .attr("d", (d) => symbolPath(d.kind, d.r))
      .style("fill", (d) => KIND_VAR[d.kind] ?? "var(--stone)");
    nodeSel
      .append("text")
      .attr("text-anchor", "middle")
      .attr("y", (d) => d.r + 4)
      .attr("dominant-baseline", "hanging")
      .text((d) => (local && nodes.length <= 10 ? d.title : shortLabel(byId.get(d.id)!)));
    nodeSelRef.current = nodeSel;
    linkSelRef.current = linkSel;

    const neighbours = new Map<string, Set<string>>();
    for (const n of nodes) neighbours.set(n.id, new Set([n.id]));
    for (const l of links) {
      const s = typeof l.source === "string" ? l.source : (l.source as SimNode).id;
      const t = typeof l.target === "string" ? l.target : (l.target as SimNode).id;
      neighbours.get(s)?.add(t);
      neighbours.get(t)?.add(s);
    }
    neighboursRef.current = neighbours;

    // Labels: always for the landmarks (crates, guides, reference, tooling); the rest when zoomed in or hovered.
    const ZOOM_LABELS = 1.5;
    let scale = 1;
    const setLabels = () => {
      nodeSel.attr("data-label", (d) =>
        (local && nodes.length <= 10) || d.kind === "crate" || d.kind === "guide" || (!local && (d.kind === "tooling" || d.kind === "reference")) || scale >= ZOOM_LABELS
          ? "shown"
          : "hidden",
      );
    };
    setLabels();

    let sim: Simulation<SimNode, SimLink>;
    if (local) {
      sim = forceSimulation<SimNode>(nodes)
        .force(
          "link",
          forceLink<SimNode, SimLink>(links)
            .distance((l) => LINK_DISTANCE[l.kind] * 0.95)
            .strength((l) => LINK_STRENGTH[l.kind]),
        )
        .force("charge", forceManyBody<SimNode>().strength(-320).distanceMax(400))
        .force("collide", forceCollide<SimNode>().radius((d) => d.r + 18))
        .force("x", forceX<SimNode>(width / 2).strength(0.08))
        .force("y", forceY<SimNode>(height / 2).strength(0.1))
        .stop();
    } else {
      const targets = structuralTargets(pages);
      const target = (d: SimNode) => targets.get(d.id) ?? { x: 0, y: 0, band: "middle" as const };
      // Only structure moves nodes: a page to its parent, a crate to the crates it depends on.
      const layoutLinks: SimLink[] = [];
      for (const p of pages) {
        const n = nodeById.get(p.id);
        if (!n) continue;
        if (p.parent && nodeById.has(p.parent)) layoutLinks.push({ source: n, target: nodeById.get(p.parent)!, kind: "parent" });
        if (p.kind === "crate") for (const d of p.depends) if (nodeById.has(d)) layoutLinks.push({ source: n, target: nodeById.get(d)!, kind: "depends" });
      }
      for (const n of nodes) {
        if (n.x == null) {
          const t = target(n);
          // Seed near the target with a deterministic jitter so the layout is the same on every load.
          const h = [...n.id].reduce((a, c) => (a * 31 + c.charCodeAt(0)) >>> 0, 7);
          n.x = t.x + ((h % 97) - 48);
          n.y = t.y + (((h >> 7) % 97) - 48);
        }
      }
      sim = forceSimulation<SimNode>(nodes)
        .force(
          "link",
          forceLink<SimNode, SimLink>(layoutLinks)
            .distance((l) => (l.kind === "parent" ? 58 : 240))
            .strength((l) => (l.kind === "parent" ? 0.7 : 0.03)),
        )
        .force("charge", forceManyBody<SimNode>().strength(-210).distanceMax(420))
        .force("collide", forceCollide<SimNode>().radius((d) => d.r + 12))
        .force("x", forceX<SimNode>((d) => target(d).x).strength((d) => (d.kind === "crate" ? 0.6 : target(d).band === "middle" ? 0.08 : 0.3)))
        .force("y", forceY<SimNode>((d) => target(d).y).strength((d) => (target(d).band === "middle" ? (d.kind === "crate" ? 0.02 : 0.01) : 0.35)))
        .stop();
    }
    simRef.current = sim;

    const render = () => {
      linkSel
        .attr("x1", (d) => (d.source as SimNode).x ?? 0)
        .attr("y1", (d) => (d.source as SimNode).y ?? 0)
        .attr("x2", (d) => (d.target as SimNode).x ?? 0)
        .attr("y2", (d) => (d.target as SimNode).y ?? 0);
      nodeSel.attr("transform", (d) => `translate(${d.x ?? 0},${d.y ?? 0})`);
    };

    // Settle the layout before the first paint so the graph does not fly in.
    const warm = previous.size === 0 || nodes.some((n) => !previous.has(n.id)) ? 360 : 60;
    for (let i = 0; i < warm; i++) sim.tick();
    render();
    sim.on("tick", render);

    const zoom = d3zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.15, 4])
      .on("zoom", (event) => {
        root.attr("transform", event.transform.toString());
        const k = event.transform.k;
        // Keep labels readable when zoomed out: grow them in graph units as the zoom shrinks (up to 1.6×).
        svgEl.style.setProperty("--g-label", `${(11 / Math.max(k, 11 / 17.6)).toFixed(2)}px`);
        if ((k >= 1.35) !== (scale >= 1.35)) {
          scale = k;
          setLabels();
        } else scale = k;
        setTip(null);
      });
    zoomRef.current = zoom;
    svg.call(zoom).on("dblclick.zoom", null);

    const fit = (animate: boolean) => {
      if (nodes.length === 0) return;
      const xs = nodes.map((n) => n.x ?? 0);
      const ys = nodes.map((n) => n.y ?? 0);
      const pad = local ? 46 : 70;
      const minX = Math.min(...xs) - pad;
      const maxX = Math.max(...xs) + pad;
      const minY = Math.min(...ys) - pad;
      const maxY = Math.max(...ys) + pad + 14;
      const w = stage.clientWidth || width;
      const h = stage.clientHeight || height;
      const k = Math.min(2, Math.max(0.15, Math.min(w / (maxX - minX), h / (maxY - minY))));
      const t = zoomIdentity.translate(w / 2 - (k * (minX + maxX)) / 2, h / 2 - (k * (minY + maxY)) / 2).scale(k);
      if (animate) svg.transition().duration(350).call(zoom.transform, t);
      else svg.call(zoom.transform, t);
    };
    fit(false);
    (svgEl as unknown as { __fit?: (a: boolean) => void }).__fit = fit;

    const near = (id: string | null) => {
      const set = id ? neighbours.get(id) : null;
      stage.dataset.hover = set ? "true" : "false";
      nodeSel.attr("data-near", (d) => (set?.has(d.id) ? "true" : null));
      linkSel.attr("data-near", (d) =>
        id && ((d.source as SimNode).id === id || (d.target as SimNode).id === id) ? "true" : null,
      );
    };

    nodeSel
      .on("pointerenter", (event: PointerEvent, d) => {
        near(d.id);
        const box = stage.getBoundingClientRect();
        setTip({ id: d.id, x: event.clientX - box.left, y: event.clientY - box.top });
      })
      .on("pointermove", (event: PointerEvent, d) => {
        const box = stage.getBoundingClientRect();
        setTip({ id: d.id, x: event.clientX - box.left, y: event.clientY - box.top });
      })
      .on("pointerleave", () => {
        near(null);
        setTip(null);
      })
      .on("focus", (_e, d) => near(d.id))
      .on("blur", () => near(null))
      .on("click", (event: MouseEvent, d) => {
        event.stopPropagation();
        if (local) onOpenRef.current?.(d.id);
        else onSelectRef.current?.(d.id);
      })
      .on("dblclick", (event: MouseEvent, d) => {
        event.stopPropagation();
        onOpenRef.current?.(d.id);
      })
      .on("keydown", (event: KeyboardEvent, d) => {
        if (event.key === "Enter") onOpenRef.current?.(d.id);
        else if (event.key === " ") {
          event.preventDefault();
          onSelectRef.current?.(d.id);
        }
      });

    nodeSel.call(
      d3drag<SVGGElement, SimNode>()
        .on("start", (event, d) => {
          if (!event.active) sim.alphaTarget(0.25).restart();
          d.fx = d.x;
          d.fy = d.y;
        })
        .on("drag", (event, d) => {
          d.fx = event.x;
          d.fy = event.y;
          setTip(null);
        })
        .on("end", (event, d) => {
          if (!event.active) sim.alphaTarget(0);
          d.fx = null;
          d.fy = null;
        }),
    );

    const onResize = () => fit(false);
    const ro = new ResizeObserver(onResize);
    ro.observe(stage);
    return () => {
      ro.disconnect();
      sim.stop();
      svg.on(".zoom", null);
    };
  }, [pages, edges, local]);

  // Focus and search matches only change attributes. In the global graph a
  // selected node also dims everything that is not its neighbour.
  useEffect(() => {
    const sel = nodeSelRef.current;
    const linkSel = linkSelRef.current;
    const stage = stageRef.current;
    if (!sel || !linkSel || !stage) return;
    sel.attr("data-focus", (d) => (d.id === focus ? "true" : null));
    sel.attr("data-match", (d) => (matches?.has(d.id) && d.id !== focus ? "true" : null));
    if (focus) sel.filter((d) => d.id === focus).raise();
    const near = !local && focus ? neighboursRef.current.get(focus) : undefined;
    stage.dataset.select = near ? "true" : "false";
    sel.attr("data-sel-near", (d) => (near?.has(d.id) ? "true" : null));
    linkSel.attr("data-sel-near", (d) =>
      focus && ((d.source as SimNode).id === focus || (d.target as SimNode).id === focus) ? "true" : null,
    );
  }, [focus, matches, pages, edges, local]);

  useImperativeHandle(
    ref,
    () => ({
      fit: () => (svgRef.current as unknown as { __fit?: (a: boolean) => void })?.__fit?.(true),
      reheat: () => {
        const sim = simRef.current;
        if (!sim) return;
        for (const n of nodesRef.current) {
          n.x = undefined;
          n.y = undefined;
          n.vx = 0;
          n.vy = 0;
        }
        sim.nodes(nodesRef.current);
        sim.alpha(1).restart();
        window.setTimeout(() => (svgRef.current as unknown as { __fit?: (a: boolean) => void })?.__fit?.(true), 900);
      },
      centerOn: (id: string) => {
        const svgEl = svgRef.current;
        const stage = stageRef.current;
        const zoom = zoomRef.current;
        const n = nodesRef.current.find((x) => x.id === id);
        if (!svgEl || !stage || !zoom || !n) return;
        const k = 1.6;
        const t = zoomIdentity
          .translate(stage.clientWidth / 2 - k * (n.x ?? 0), stage.clientHeight / 2 - k * (n.y ?? 0))
          .scale(k);
        select(svgEl).transition().duration(450).call(zoom.transform, t);
      },
      zoomBy: (factor: number) => {
        const svgEl = svgRef.current;
        const zoom = zoomRef.current;
        if (svgEl && zoom) select(svgEl).transition().duration(200).call(zoom.scaleBy, factor);
      },
    }),
    [],
  );

  const tipPage = tip ? byId.get(tip.id) : null;
  return (
    <div ref={stageRef} className={`graph-stage${local ? " graph-stage--local" : ""}`} data-hover="false">
      <svg ref={svgRef} role="img" aria-label={ariaLabel} onClick={() => onSelectRef.current && !local && onSelectRef.current("")} />
      {tipPage && tip ? (
        <div
          className="graph-tip"
          style={{
            left: Math.min(tip.x + 14, (stageRef.current?.clientWidth ?? 600) - 290),
            top: Math.max(tip.y - 10, 8),
          }}
        >
          <strong>{tipPage.title}</strong>
          <span className="tag" data-kind={tipPage.kind} style={{ margin: "4px 0" }}>
            {KIND_LABEL[tipPage.kind]}
          </span>
          <div className="muted">{tipPage.summary}</div>
        </div>
      ) : null}
    </div>
  );
});

/** Small legend item: the node shape in its kind colour. */
export function KindGlyph({ kind, size = 14 }: { kind: Kind; size?: number }) {
  const d = symbolPath(kind, 5.2);
  return (
    <svg width={size} height={size} viewBox="-8 -8 16 16" aria-hidden="true" style={{ overflow: "visible" }}>
      <path d={d} style={{ fill: KIND_VAR[kind] }} stroke="#141111" strokeWidth={1.6} />
    </svg>
  );
}
