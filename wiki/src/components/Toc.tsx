import { useEffect, useState } from "react";
import type { Page } from "../core/wiki.js";
import { pageHref } from "../lib/router";

/** Section list for the current page; the section in view is marked. */
export function Toc({ page }: { page: Page }) {
  const items = page.headings.filter((h) => h.depth === 2 || h.depth === 3);
  const [active, setActive] = useState<string | null>(null);

  useEffect(() => {
    setActive(null);
    const els = items.map((h) => document.getElementById(h.slug)).filter((el): el is HTMLElement => Boolean(el));
    if (els.length === 0) return;
    const visible = new Map<string, number>();
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) visible.set(e.target.id, e.boundingClientRect.top);
          else visible.delete(e.target.id);
        }
        if (visible.size > 0) {
          const first = [...visible.entries()].sort((a, b) => a[1] - b[1])[0][0];
          setActive(first);
        } else {
          // Between headings: the last heading above the top edge is the current section.
          const above = els.filter((el) => el.getBoundingClientRect().top < 120);
          if (above.length) setActive(above[above.length - 1].id);
        }
      },
      { rootMargin: "-70px 0px -55% 0px" },
    );
    els.forEach((el) => io.observe(el));
    return () => io.disconnect();
    // items derive from page; re-run when the page changes
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page]);

  if (items.length === 0) return <p className="muted" style={{ padding: "10px 14px", margin: 0 }}>这一页没有小节。</p>;
  return (
    <ul className="toc">
      {items.map((h) => (
        <li key={h.slug} className={h.depth === 3 ? "d3" : undefined}>
          <a href={pageHref(page.id, h.slug)} data-active={active === h.slug}>
            {h.text}
          </a>
        </li>
      ))}
    </ul>
  );
}
