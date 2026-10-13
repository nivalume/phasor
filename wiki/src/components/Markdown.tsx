import { memo, useState, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeSlug from "rehype-slug";
import rehypeHighlight from "rehype-highlight";
import type { Element, ElementContent, Root, RootContent } from "hast";
import { resolveCodePath, resolveHref, type Page } from "../core/wiki.js";
import { REPO_URL, CONTENT_REF } from "../config";
import { getPage, imageUrls } from "../lib/data";
import { githubUrl } from "../lib/github";
import { repoExists } from "../lib/repo";
import { pageHref } from "../lib/router";
import { Mermaid } from "./Mermaid";

/** Plain text of a hast subtree. */
function textOf(node: Root | RootContent | ElementContent): string {
  if (node.type === "text") return node.value;
  if ("children" in node) return (node.children as (RootContent | ElementContent)[]).map(textOf).join("");
  return "";
}

/**
 * Inline code that names a repository file (`crates/barq-exec/src/kernel.rs:144`,
 * or `src/lib.rs` on a crate page) becomes a link to that file and line on
 * GitHub, but only when the file exists. Runs before rendering so it can skip
 * code that is already inside a link or a code block.
 */
function rehypeRepoCode(options: { page: Page }) {
  return (tree: Root) => {
    const walk = (node: Root | Element, inLinkOrPre: boolean) => {
      node.children.forEach((child, i) => {
        if (child.type !== "element") return;
        const blocked = inLinkOrPre || child.tagName === "a" || child.tagName === "pre";
        if (!blocked && child.tagName === "code") {
          const ref = resolveCodePath(textOf(child), options.page, repoExists);
          if (ref) {
            node.children[i] = {
              type: "element",
              tagName: "a",
              properties: { href: githubUrl(ref), className: ["code-link"], dataRepo: "true", title: ref },
              children: [child],
            };
            return;
          }
        }
        walk(child, blocked);
      });
    };
    walk(tree, false);
  };
}

function languageOf(node: Element | undefined): string | null {
  const cls: unknown = node?.properties?.className;
  const list: string[] = Array.isArray(cls) ? cls.map(String) : typeof cls === "string" ? cls.split(" ") : [];
  const lang = list.find((c) => c.startsWith("language-"));
  return lang ? lang.slice("language-".length) : null;
}

function CodeBlock({ lang, text, children }: { lang: string | null; text: string; children: ReactNode }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text.replace(/\n$/, ""));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    } catch {
      /* clipboard blocked: nothing to do, the text is selectable */
    }
  };
  return (
    <div className="codeblock">
      <div className="codeblock__bar">
        <span>{lang ?? "text"}</span>
        <button type="button" className="codeblock__copy" onClick={copy}>
          {copied ? "已复制" : "复制"}
        </button>
        <span className="window__dots" aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
      </div>
      <pre>{children}</pre>
    </div>
  );
}

function makeComponents(page: Page): Components {
  const heading =
    (Tag: "h2" | "h3" | "h4") =>
    ({ id, children }: { id?: string; children?: ReactNode }) => (
      <Tag id={id}>
        {children}
        {id ? (
          <a className="anchor" href={pageHref(page.id, id)} aria-label="链接到本节">
            #
          </a>
        ) : null}
      </Tag>
    );

  return {
    h2: heading("h2"),
    h3: heading("h3"),
    h4: heading("h4"),

    a({ href = "", children, className, node }) {
      if (node?.properties?.dataRepo) {
        return (
          <a className={className} href={href} target="_blank" rel="noreferrer" title={`在 GitHub 上打开 ${node.properties.title ?? ""}`}>
            {children}
          </a>
        );
      }
      const target = resolveHref(page.path, href);
      switch (target.type) {
        case "page": {
          const linked = getPage(target.id);
          if (!linked) {
            return (
              <a href={href} data-missing="true" title={`页面不存在：${target.id}`}>
                {children}
              </a>
            );
          }
          return (
            <a href={pageHref(linked.id, target.anchor)} className={className} title={linked.summary} data-kind={linked.kind}>
              {children}
            </a>
          );
        }
        case "anchor":
          return (
            <a href={pageHref(page.id, target.anchor)} className={className}>
              {children}
            </a>
          );
        case "repo": {
          const url = githubUrl(target.path) + (target.anchor ? `#${target.anchor}` : "");
          return (
            <a href={url} className={className} target="_blank" rel="noreferrer" data-external="true">
              {children}
            </a>
          );
        }
        case "external":
          return (
            <a href={target.url} className={className} target="_blank" rel="noreferrer" data-external="true">
              {children}
            </a>
          );
        default:
          return (
            <a href={href} data-missing="true">
              {children}
            </a>
          );
      }
    },

    img({ src = "", alt = "" }) {
      const target = resolveHref(page.path, src);
      let url = src;
      if (target.type === "repo") url = imageUrls.get(target.path) ?? `${REPO_URL}/raw/${CONTENT_REF}/${target.path}`;
      else if (target.type === "external") url = target.url;
      return <img src={url} alt={alt} loading="lazy" />;
    },

    pre({ node, children }) {
      const code = node?.children.find((c): c is Element => c.type === "element" && c.tagName === "code");
      const lang = languageOf(code);
      const text = code ? textOf(code) : "";
      if (lang === "mermaid") return <Mermaid code={text.replace(/\n$/, "")} />;
      return (
        <CodeBlock lang={lang} text={text}>
          {children}
        </CodeBlock>
      );
    },

    table({ children }) {
      return (
        <div className="table-wrap">
          <table>{children}</table>
        </div>
      );
    },
  };
}

/** One page body rendered as HTML. Memoised: pages are immutable. */
export const Markdown = memo(function Markdown({ page }: { page: Page }) {
  return (
    <div className="prose">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeSlug, [rehypeHighlight, { detect: false, plainText: ["mermaid", "text"] }], [rehypeRepoCode, { page }]]}
        components={makeComponents(page)}
      >
        {page.body}
      </ReactMarkdown>
    </div>
  );
});
