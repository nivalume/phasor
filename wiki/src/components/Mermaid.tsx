import { useEffect, useId, useState } from "react";

type MermaidApi = typeof import("mermaid").default;
let loader: Promise<MermaidApi> | null = null;

/**
 * Mermaid is ~2 MB, so it is loaded on first use. Diagrams always render in
 * the light palette on a white card, like the architecture SVGs: many blocks
 * set their own classDef fills, which only read with dark text.
 */
function loadMermaid(): Promise<MermaidApi> {
  loader ??= import("mermaid").then((m) => {
    const mermaid = m.default;
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      theme: "base",
      fontFamily: '"Space Grotesk", "PingFang SC", "Noto Sans SC", system-ui, sans-serif',
      themeVariables: {
        background: "#ffffff",
        primaryColor: "#fffaef",
        primaryTextColor: "#141111",
        primaryBorderColor: "#141111",
        secondaryColor: "#ffd440",
        tertiaryColor: "#f1ebdd",
        lineColor: "#141111",
        textColor: "#141111",
        noteBkgColor: "#ffe58f",
        noteBorderColor: "#141111",
        actorBkg: "#fffaef",
        actorBorder: "#141111",
        clusterBkg: "#fbf1da",
        clusterBorder: "#141111",
        edgeLabelBackground: "#ffffff",
        fontSize: "14px",
      },
      flowchart: { htmlLabels: true, curve: "basis" },
      sequence: { useMaxWidth: true },
      class: { useMaxWidth: true },
    });
    return mermaid;
  });
  return loader;
}

export function Mermaid({ code }: { code: string }) {
  const reactId = useId();
  const id = `mmd-${reactId.replace(/[^a-zA-Z0-9]/g, "")}`;
  const [svg, setSvg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const kind = code.trim().split(/\s+/)[0] ?? "mermaid";

  useEffect(() => {
    let cancelled = false;
    setSvg(null);
    setError(null);
    loadMermaid()
      .then((mermaid) => mermaid.render(id, code))
      .then(({ svg }) => {
        if (!cancelled) setSvg(svg);
      })
      .catch((e: unknown) => {
        document.getElementById(`d${id}`)?.remove();
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [code, id]);

  return (
    <figure className="diagram" aria-label={`${kind} 图`}>
      <div className="diagram__bar">
        <span>mermaid · {kind}</span>
        <span className="window__dots" aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
      </div>
      {error ? (
        <div className="diagram__error">图表渲染失败：{error}</div>
      ) : svg ? (
        // The SVG comes from mermaid.render with securityLevel "strict", which sanitises labels.
        <div className="diagram__canvas" dangerouslySetInnerHTML={{ __html: svg }} />
      ) : (
        <div className="diagram__canvas muted">正在绘制…</div>
      )}
    </figure>
  );
}
