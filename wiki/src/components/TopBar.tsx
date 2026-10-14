import { REPO_URL } from "../config";
import type { Route } from "../lib/router";
import type { Theme } from "../lib/theme";

export function TopBar({
  route,
  theme,
  onToggleTheme,
  onSearch,
  onMenu,
  menuOpen,
}: {
  route: Route;
  theme: Theme;
  onToggleTheme: () => void;
  onSearch: () => void;
  onMenu?: () => void;
  menuOpen: boolean;
}) {
  const at = (id: string) => (route.name === "page" && route.id === id ? "page" : undefined);
  const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);
  return (
    <header className="topbar">
      {onMenu ? (
        <button
          type="button"
          className="btn btn--small btn--square menu-button"
          onClick={onMenu}
          aria-label={menuOpen ? "关闭目录" : "打开目录"}
          aria-expanded={menuOpen}
          aria-controls="side-nav"
        >
          {menuOpen ? "✕" : "☰"}
        </button>
      ) : null}
      <a className="brand" href="#/" aria-label="phasor 知识库首页">
        <span className="brand__mark">phasor</span>
        <span>知识库</span>
        <span className="brand__sub">/ WIKI</span>
      </a>
      <nav className="topbar__nav" aria-label="主导航">
        <a className="topbar__link" href="#/p/reading-roadmap" aria-current={at("reading-roadmap")}>
          阅读路线
        </a>
        <a className="topbar__link" href="#/p/system-map" aria-current={at("system-map")}>
          系统全景
        </a>
        <a className="topbar__link" href="#/p/algorithm-index" aria-current={at("algorithm-index")}>
          算法一览
        </a>
        <a className="topbar__link" href="#/graph" aria-current={route.name === "graph" ? "page" : undefined}>
          知识图谱
        </a>
        <button type="button" className="btn btn--small" onClick={onSearch} aria-label="搜索">
          <span aria-hidden="true">⌕</span>
          <span className="hide-narrow">搜索</span>
          <span className="kbd hide-narrow">{isMac ? "⌘K" : "Ctrl K"}</span>
        </button>
        <button
          type="button"
          className="btn btn--small btn--square"
          onClick={onToggleTheme}
          aria-label={theme === "dark" ? "切换到浅色" : "切换到深色"}
          title={theme === "dark" ? "浅色" : "深色"}
        >
          {theme === "dark" ? "☀" : "☾"}
        </button>
        <a className="btn btn--small btn--pink hide-narrow" href={REPO_URL} target="_blank" rel="noreferrer">
          GitHub ↗
        </a>
      </nav>
    </header>
  );
}
