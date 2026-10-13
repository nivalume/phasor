import { useEffect, useState } from "react";

/**
 * Hash routes, so the built site works from any static host or sub-path.
 *   #/                 home
 *   #/p/<id>           a page
 *   #/p/<id>/<slug>    a page, scrolled to a heading
 *   #/graph            the knowledge graph
 *   #/graph/<id>       the graph, with a page selected
 */
export type Route =
  | { name: "home" }
  | { name: "page"; id: string; anchor: string | null }
  | { name: "graph"; focus: string | null }
  | { name: "missing"; path: string };

export function parseHash(hash: string): Route {
  const path = hash.replace(/^#/, "") || "/";
  const parts = path.split("/").filter(Boolean).map((s) => decodeURIComponent(s));
  if (parts.length === 0) return { name: "home" };
  if (parts[0] === "p" && parts[1]) return { name: "page", id: parts[1], anchor: parts[2] ?? null };
  if (parts[0] === "graph") return { name: "graph", focus: parts[1] ?? null };
  return { name: "missing", path };
}

export function pageHref(id: string, anchor?: string | null): string {
  return `#/p/${encodeURIComponent(id)}${anchor ? `/${encodeURIComponent(anchor)}` : ""}`;
}

export function graphHref(focus?: string | null): string {
  return `#/graph${focus ? `/${encodeURIComponent(focus)}` : ""}`;
}

export function navigate(href: string) {
  if (window.location.hash !== href) window.location.hash = href;
}

/** Update the address without triggering navigation (used for in-page anchors). */
export function replaceHash(href: string) {
  history.replaceState(null, "", href);
}

export function useRoute(): Route {
  const [route, setRoute] = useState<Route>(() => parseHash(window.location.hash));
  useEffect(() => {
    const onChange = () => setRoute(parseHash(window.location.hash));
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);
  return route;
}
