import { useCallback, useEffect, useState } from "react";
import { CommandPalette } from "./components/CommandPalette";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { CODE_REF, REPO_URL } from "./config";
import { wiki } from "./lib/data";
import { useRoute } from "./lib/router";
import { useTheme } from "./lib/theme";
import { GraphPage } from "./pages/GraphPage";
import { HomePage } from "./pages/HomePage";
import { NotFound } from "./pages/NotFound";
import { PageRail, WikiPage } from "./pages/WikiPage";

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  return Boolean(el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable));
}

export function App() {
  const route = useRoute();
  const [theme, toggleTheme] = useTheme();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);

  const openPalette = useCallback(() => {
    setMenuOpen(false);
    setPaletteOpen(true);
  }, []);

  // ⌘K / Ctrl+K anywhere, "/" when not typing: open search.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      } else if (e.key === "/" && !paletteOpen && !isTyping(e.target)) {
        e.preventDefault();
        openPalette();
      } else if (e.key === "Escape") setMenuOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [paletteOpen, openPalette]);

  // Navigating closes the drawer.
  useEffect(() => setMenuOpen(false), [route]);

  const page = route.name === "page" ? wiki.pages.get(route.id) : undefined;

  let content;
  if (route.name === "page" && page) {
    content = (
      <div className="shell">
        <aside id="side-nav" className="shell__side" data-open={menuOpen} aria-label="页面目录">
          <Sidebar current={page.id} onSearch={openPalette} />
        </aside>
        <div className="drawer-backdrop" data-open={menuOpen} onClick={() => setMenuOpen(false)} aria-hidden="true" />
        <main id="main" className="shell__main">
          <WikiPage page={page} anchor={route.anchor} />
        </main>
        <aside className="shell__rail" aria-label="本页关系">
          <PageRail page={page} />
        </aside>
      </div>
    );
  } else if (route.name === "page") {
    content = (
      <main id="main">
        <NotFound what={route.id} />
      </main>
    );
  } else if (route.name === "graph") {
    content = (
      <main id="main">
        <GraphPage focus={route.focus} />
      </main>
    );
  } else if (route.name === "home") {
    content = (
      <main id="main">
        <HomePage />
      </main>
    );
  } else {
    content = (
      <main id="main">
        <NotFound what={route.path} />
      </main>
    );
  }

  return (
    <>
      <TopBar
        route={route}
        theme={theme}
        onToggleTheme={toggleTheme}
        onSearch={openPalette}
        onMenu={route.name === "page" && page ? () => setMenuOpen((o) => !o) : undefined}
        menuOpen={menuOpen}
      />
      {content}
      {route.name !== "graph" ? (
        <footer className="site-foot">
          <span>phasor 知识库 · {wiki.pages.size} 页 · 内容在 wiki/content/</span>
          <span>
            代码行号对应提交{" "}
            <a href={`${REPO_URL}/tree/${CODE_REF}`} target="_blank" rel="noreferrer">
              {CODE_REF.slice(0, 7)}
            </a>
          </span>
        </footer>
      ) : null}
      {paletteOpen ? <CommandPalette onClose={() => setPaletteOpen(false)} /> : null}
    </>
  );
}
