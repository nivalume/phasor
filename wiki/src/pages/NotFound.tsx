import { useEffect } from "react";
import { Window } from "../components/Window";
import { graphHref, pageHref } from "../lib/router";

export function NotFound({ what }: { what: string }) {
  useEffect(() => {
    document.title = "找不到页面 · phasor 知识库";
  }, []);
  return (
    <div className="empty-state">
      <Window icon="?" iconColor="var(--red)" title="404" subtitle={what} dotted>
        <div className="window__body">
          <p style={{ marginTop: 0 }}>
            知识库里没有 <code className="mono">{what}</code>。页面可能改了名字，或者链接写错了。
          </p>
          <div className="hero__cta">
            <a className="btn btn--pink" href="#/">
              回到首页
            </a>
            <a className="btn" href={pageHref("reading-roadmap")}>
              阅读路线
            </a>
            <a className="btn" href={graphHref()}>
              知识图谱
            </a>
          </div>
        </div>
      </Window>
    </div>
  );
}
