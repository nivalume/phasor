import { useMemo } from "react";
import type { Kind } from "../core/wiki.js";
import { KIND_VAR, PIXELS } from "../lib/kinds";

/** One 8×8 pixel icon on its kind colour; `online` adds the green presence dot. */
export function PixelAvatar({ kind, online = false, size = 34, title }: { kind: Kind; online?: boolean; size?: number; title?: string }) {
  const d = useMemo(() => {
    const rows = PIXELS[kind] ?? PIXELS.concept;
    let path = "";
    rows.forEach((row, y) => {
      for (let x = 0; x < row.length; x++) if (row[x] === "#") path += `M${x} ${y}h1v1h-1z`;
    });
    return path;
  }, [kind]);
  return (
    <span className={`avatar${online ? " avatar--online" : ""}`} style={{ width: size, height: size }} title={title} aria-hidden={title ? undefined : true}>
      <svg viewBox="-1 -1 10 10" shapeRendering="crispEdges">
        <rect x="-1" y="-1" width="10" height="10" fill={KIND_VAR[kind] ?? "var(--stone)"} />
        <path d={d} fill="#141111" />
      </svg>
    </span>
  );
}
