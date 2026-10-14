// @ts-check
/**
 * Content model of the phasor wiki, shared by the React app (through Vite) and by
 * the Node checker (scripts/check-wiki.mjs). Plain ESM JavaScript with JSDoc
 * types so both can import it without a build step.
 *
 * A page is one Markdown file under wiki/content/. Its id is the file name
 * without `.md`, unique across all sections. Pages relate in four ways, and
 * every one of them becomes a typed edge of the knowledge graph:
 *
 *   link     a Markdown link in the body to another page
 *   parent   `parent:` in the frontmatter (an algorithm or concept belongs to a crate or tooling page)
 *   depends  `depends:` on a crate page (crate → crate, from Cargo.toml)
 *   related  `related:` in the frontmatter (see-also, not directional)
 */

import { parse as parseYaml } from "yaml";
import GithubSlugger from "github-slugger";

/** @typedef {"guide" | "crate" | "concept" | "algorithm" | "reference" | "tooling"} Kind */
/** @typedef {"link" | "parent" | "depends" | "related"} EdgeKind */

/** Page kinds in display order, with their Chinese labels. */
export const KINDS = /** @type {const} */ ([
  { id: "guide", label: "指南", plural: "指南" },
  { id: "crate", label: "Crate", plural: "Crate" },
  { id: "concept", label: "概念", plural: "概念" },
  { id: "algorithm", label: "算法", plural: "算法" },
  { id: "reference", label: "参考", plural: "参考" },
  { id: "tooling", label: "工具链", plural: "工具链" },
]);

export const KIND_IDS = KINDS.map((k) => k.id);

/** Edge kinds with their Chinese labels. */
export const EDGE_KINDS = /** @type {const} */ ([
  { id: "link", label: "正文链接" },
  { id: "parent", label: "所属" },
  { id: "depends", label: "依赖" },
  { id: "related", label: "相关" },
]);

/** Where the wiki lives inside the repository. */
export const CONTENT_ROOT = "wiki/content";

/**
 * @typedef {object} Heading
 * @property {number} depth
 * @property {string} text
 * @property {string} slug
 */

/**
 * @typedef {object} Link
 * @property {string} href      the raw href as written
 * @property {string} text      link text (or image alt)
 * @property {boolean} image    whether this is `![…](…)`
 * @property {number} line      1-based line in the file
 * @property {Resolved} target
 */

/**
 * @typedef {{ type: "page", id: string, anchor: string | null }
 *   | { type: "repo", path: string, anchor: string | null }
 *   | { type: "external", url: string }
 *   | { type: "anchor", anchor: string }
 *   | { type: "invalid", reason: string }} Resolved
 */

/**
 * @typedef {object} Page
 * @property {string} id
 * @property {string} path         repo-relative path, e.g. wiki/content/algorithms/size-cascade.md
 * @property {string} section      first directory under content/, "" for index.md
 * @property {string} title
 * @property {Kind} kind
 * @property {string} summary
 * @property {string | null} parent
 * @property {string[]} depends
 * @property {string[]} related
 * @property {string[]} tags
 * @property {string[]} sources    repo paths, optionally with :line or :a-b
 * @property {number} order
 * @property {string} body         Markdown without the frontmatter
 * @property {number} bodyLine     1-based line where the body starts
 * @property {Heading[]} headings
 * @property {Link[]} links
 * @property {Record<string, unknown>} data   the raw frontmatter
 */

/**
 * @typedef {object} Edge
 * @property {string} source
 * @property {string} target
 * @property {EdgeKind} kind
 */

/**
 * @typedef {object} Problem
 * @property {string} path
 * @property {number} line
 * @property {"error" | "warning"} level
 * @property {string} message
 */

/**
 * @typedef {object} Wiki
 * @property {Map<string, Page>} pages
 * @property {Edge[]} edges
 * @property {Map<string, Edge[]>} outgoing
 * @property {Map<string, Edge[]>} incoming
 * @property {Problem[]} problems
 */

/**
 * Normalise a POSIX path: collapse `.` and `..`, keep it relative.
 * @param {string} path
 */
export function normalizePath(path) {
  const out = [];
  for (const part of path.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") {
      if (out.length === 0 || out[out.length - 1] === "..") out.push("..");
      else out.pop();
    } else out.push(part);
  }
  return out.join("/");
}

/**
 * Directory part of a POSIX path.
 * @param {string} path
 */
export function dirname(path) {
  const i = path.lastIndexOf("/");
  return i < 0 ? "" : path.slice(0, i);
}

/**
 * File name without `.md`.
 * @param {string} path
 */
export function pageIdFromPath(path) {
  const base = path.slice(path.lastIndexOf("/") + 1);
  return base.replace(/\.md$/, "");
}

/**
 * Split YAML frontmatter from the body.
 * @param {string} text
 * @returns {{ data: Record<string, unknown>, body: string, bodyLine: number, error: string | null }}
 */
export function splitFrontmatter(text) {
  const normalized = text.replace(/\r\n/g, "\n");
  if (!normalized.startsWith("---\n")) {
    return { data: {}, body: normalized, bodyLine: 1, error: "missing frontmatter (the file must start with ---)" };
  }
  const end = normalized.indexOf("\n---", 4);
  if (end < 0) return { data: {}, body: normalized, bodyLine: 1, error: "unterminated frontmatter" };
  const yamlText = normalized.slice(4, end);
  const afterFence = normalized.indexOf("\n", end + 1);
  const body = afterFence < 0 ? "" : normalized.slice(afterFence + 1);
  const bodyLine = yamlText.split("\n").length + 3;
  try {
    const data = parseYaml(yamlText) ?? {};
    if (typeof data !== "object" || Array.isArray(data)) {
      return { data: {}, body, bodyLine, error: "frontmatter must be a mapping" };
    }
    return { data: /** @type {Record<string, unknown>} */ (data), body, bodyLine, error: null };
  } catch (e) {
    return { data: {}, body, bodyLine, error: `invalid YAML: ${e instanceof Error ? e.message : String(e)}` };
  }
}

/**
 * Replace fenced code blocks and inline code with spaces so link and heading
 * scans skip them; line numbers are preserved.
 * @param {string} body
 */
export function maskCode(body) {
  const lines = body.split("\n");
  /** @type {string | null} */
  let fence = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const m = /^\s*(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (m && m[1][0] === fence[0] && m[1].length >= fence.length) fence = null;
      lines[i] = "";
      continue;
    }
    if (m) {
      fence = m[1];
      lines[i] = "";
      continue;
    }
    lines[i] = line.replace(/(`+)(?:(?!\1)[\s\S])*?\1/g, (s) => " ".repeat(s.length));
  }
  return lines.join("\n");
}

/**
 * Text of a heading line without Markdown decoration (for slugs).
 * @param {string} raw
 */
function headingText(raw) {
  return raw
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/`([^`]*)`/g, "$1")
    .replace(/[*_~]/g, "")
    .trim();
}

/**
 * Headings with the slugs rehype-slug will give them (github-slugger, in order).
 * @param {string} body
 * @returns {Heading[]}
 */
export function extractHeadings(body) {
  const slugger = new GithubSlugger();
  const lines = body.split("\n");
  /** @type {Heading[]} */
  const out = [];
  /** @type {string | null} */
  let fence = null;
  for (const line of lines) {
    const f = /^\s*(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
      continue;
    }
    if (f) {
      fence = f[1];
      continue;
    }
    const m = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line);
    if (!m) continue;
    const text = headingText(m[2]);
    out.push({ depth: m[1].length, text, slug: slugger.slug(text) });
  }
  return out;
}

/**
 * Resolve an href written in the file at `fromPath` (repo-relative).
 * @param {string} fromPath
 * @param {string} href
 * @returns {Resolved}
 */
export function resolveHref(fromPath, href) {
  const raw = href.trim().replace(/^<|>$/g, "");
  if (raw === "") return { type: "invalid", reason: "empty link" };
  if (/^[a-z][a-z0-9+.-]*:/i.test(raw)) return { type: "external", url: raw };
  if (raw.startsWith("#")) return { type: "anchor", anchor: decodeURIComponent(raw.slice(1)) };
  if (raw.startsWith("/")) return { type: "invalid", reason: "absolute paths do not work on GitHub; use a relative path" };
  const hashAt = raw.indexOf("#");
  const pathPart = hashAt < 0 ? raw : raw.slice(0, hashAt);
  const anchor = hashAt < 0 ? null : decodeURIComponent(raw.slice(hashAt + 1));
  const joined = normalizePath(`${dirname(fromPath)}/${decodeURI(pathPart)}`);
  if (joined.startsWith("..")) return { type: "invalid", reason: "points outside the repository" };
  if (joined.startsWith(`${CONTENT_ROOT}/`) && joined.endsWith(".md")) {
    return { type: "page", id: pageIdFromPath(joined), anchor };
  }
  return { type: "repo", path: joined, anchor };
}

/**
 * Inline links and images in a body (code masked).
 * @param {string} body
 * @param {string} fromPath
 * @param {number} bodyLine
 * @returns {Link[]}
 */
export function extractLinks(body, fromPath, bodyLine) {
  const masked = maskCode(body);
  /** @type {Link[]} */
  const out = [];
  const re = /(!?)\[((?:[^\[\]]|\[[^\]]*\])*)\]\(\s*(<[^>]*>|[^\s)]+)(?:\s+"[^"]*")?\s*\)/g;
  const lines = masked.split("\n");
  for (let i = 0; i < lines.length; i++) {
    let m;
    re.lastIndex = 0;
    while ((m = re.exec(lines[i]))) {
      out.push({
        href: m[3],
        text: m[2],
        image: m[1] === "!",
        line: bodyLine + i,
        target: resolveHref(fromPath, m[3]),
      });
    }
  }
  return out;
}

/** @param {unknown} v */
function asStringList(v) {
  if (v == null) return [];
  if (Array.isArray(v)) return v.map((x) => String(x));
  return [String(v)];
}

/**
 * Build the wiki from raw files.
 * @param {{ path: string, text: string }[]} files  repo-relative paths under wiki/content
 * @returns {Wiki}
 */
export function buildWiki(files) {
  /** @type {Map<string, Page>} */
  const pages = new Map();
  /** @type {Problem[]} */
  const problems = [];
  /** @type {(path: string, line: number, message: string) => void} */
  const err = (path, line, message) => {
    problems.push({ path, line, level: "error", message });
  };

  for (const file of [...files].sort((a, b) => a.path.localeCompare(b.path))) {
    const id = pageIdFromPath(file.path);
    const rel = file.path.slice(CONTENT_ROOT.length + 1);
    const section = rel.includes("/") ? rel.slice(0, rel.indexOf("/")) : "";
    const { data, body, bodyLine, error } = splitFrontmatter(file.text);
    if (error) err(file.path, 1, error);
    if (pages.has(id)) {
      err(file.path, 1, `duplicate page id "${id}" (also ${pages.get(id)?.path}); file names must be unique across sections`);
      continue;
    }
    const kind = /** @type {Kind} */ (String(data.kind ?? ""));
    pages.set(id, {
      id,
      path: file.path,
      section,
      title: String(data.title ?? id),
      kind,
      summary: String(data.summary ?? ""),
      parent: data.parent == null ? null : String(data.parent),
      depends: asStringList(data.depends),
      related: asStringList(data.related),
      tags: asStringList(data.tags),
      sources: asStringList(data.sources),
      order: typeof data.order === "number" ? data.order : 1000,
      body,
      bodyLine,
      headings: extractHeadings(body),
      links: extractLinks(body, file.path, bodyLine),
      data,
    });
  }

  /** @type {Edge[]} */
  const edges = [];
  const seen = new Set();
  /** @type {(source: string, target: string, kind: EdgeKind) => void} */
  const addEdge = (source, target, kind) => {
    if (source === target) return;
    const key = kind === "related" ? [kind, ...[source, target].sort()].join("\u0000") : [kind, source, target].join("\u0000");
    if (seen.has(key)) return;
    seen.add(key);
    edges.push({ source, target, kind });
  };
  for (const page of pages.values()) {
    for (const link of page.links) {
      if (link.target.type === "page" && !link.image && pages.has(link.target.id)) addEdge(page.id, link.target.id, "link");
    }
    if (page.parent && pages.has(page.parent)) addEdge(page.id, page.parent, "parent");
    for (const d of page.depends) if (pages.has(d)) addEdge(page.id, d, "depends");
    for (const r of page.related) if (pages.has(r)) addEdge(page.id, r, "related");
  }

  /** @type {Map<string, Edge[]>} */
  const outgoing = new Map();
  /** @type {Map<string, Edge[]>} */
  const incoming = new Map();
  for (const id of pages.keys()) {
    outgoing.set(id, []);
    incoming.set(id, []);
  }
  for (const e of edges) {
    outgoing.get(e.source)?.push(e);
    incoming.get(e.target)?.push(e);
  }
  return { pages, edges, outgoing, incoming, problems };
}

/**
 * Ids of all pages connected to `id` by any edge (either direction).
 * @param {Wiki} wiki
 * @param {string} id
 */
export function neighbours(wiki, id) {
  const out = new Set();
  for (const e of wiki.outgoing.get(id) ?? []) out.add(e.target);
  for (const e of wiki.incoming.get(id) ?? []) out.add(e.source);
  out.delete(id);
  return out;
}

/**
 * Ids within `depth` hops of `id`, including it.
 * @param {Wiki} wiki
 * @param {string} id
 * @param {number} depth
 */
export function neighbourhood(wiki, id, depth) {
  const seen = new Set([id]);
  let frontier = [id];
  for (let d = 0; d < depth; d++) {
    const next = [];
    for (const n of frontier) {
      for (const m of neighbours(wiki, n)) {
        if (!seen.has(m)) {
          seen.add(m);
          next.push(m);
        }
      }
    }
    frontier = next;
  }
  return seen;
}

/**
 * Repo source reference like `crates/a/b.rs:12-30` → path and line range.
 * @param {string} ref
 * @returns {{ path: string, start: number | null, end: number | null }}
 */
export function parseSourceRef(ref) {
  const m = /^(.+?)(?::(\d+)(?:-(\d+))?)?$/.exec(ref.trim());
  if (!m) return { path: ref, start: null, end: null };
  return { path: m[1], start: m[2] ? Number(m[2]) : null, end: m[3] ? Number(m[3]) : null };
}

/**
 * Whether an inline-code string looks like a path from the repository root.
 * @param {string} text
 */
export function looksLikeRepoPath(text) {
  return /^(crates|research|tests|benchmarks|examples|docs|skills|wiki)\/[\w./-]+?(:\d+(-\d+)?)?$/.test(text.trim());
}

/**
 * Whether an inline-code string could name a file at all: `a/b.rs`, `src/lib.rs:12-30`, `dir/`.
 * @param {string} text
 */
export function looksLikePath(text) {
  return /^[\w.@-]+(?:\/[\w.@-]+)*\/[\w.@-]*(?::\d+(?:-\d+)?)?$/.test(text.trim());
}

/**
 * The crate directory a page's relative paths are written against: a crate
 * page's own crate, or the crate its parent is. `null` for other pages.
 * @param {Page | null | undefined} page
 */
export function crateContext(page) {
  if (!page) return null;
  if (page.kind === "crate") return page.id;
  if (page.parent && page.parent.startsWith("phasor")) return page.parent;
  return null;
}

/**
 * Resolve inline code that names a repository file. Pages often write paths
 * relative to their crate (`src/lib.rs`, `tests/parity_signals.rs`) or to
 * `crates/` (`phasor-py/src/array.rs`); try, in order, the path as written, the
 * page's crate directory, and `crates/`. Returns the repo-relative path with
 * any `:line` suffix kept, or null when no candidate exists.
 * @param {string} text
 * @param {Page | null | undefined} page
 * @param {(path: string) => boolean} exists  true for files and directories
 * @returns {string | null}
 */
export function resolveCodePath(text, page, exists) {
  const raw = text.trim();
  if (!looksLikePath(raw)) return null;
  const { path, start, end } = parseSourceRef(raw);
  const suffix = start == null ? "" : end == null ? `:${start}` : `:${start}-${end}`;
  const crate = crateContext(page);
  const candidates = [path];
  if (crate) candidates.push(`crates/${crate}/${path}`);
  if (/^phasor(-[\w]+)?\//.test(path)) candidates.push(`crates/${path}`);
  for (const c of candidates) {
    const normalized = normalizePath(c) + (c.endsWith("/") ? "/" : "");
    if (!normalized.startsWith("..") && exists(normalized.replace(/\/$/, ""))) return normalized + suffix;
  }
  return null;
}

/**
 * Inline code spans outside fenced blocks, with their 1-based file line.
 * @param {string} body
 * @param {number} bodyLine
 * @returns {{ text: string, line: number }[]}
 */
export function extractInlineCode(body, bodyLine) {
  /** @type {{ text: string, line: number }[]} */
  const out = [];
  /** @type {string | null} */
  let fence = null;
  body.split("\n").forEach((line, i) => {
    const f = /^\s*(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
      return;
    }
    if (f) {
      fence = f[1];
      return;
    }
    for (const m of line.matchAll(/(`+)((?:(?!\1)[\s\S])*?)\1/g)) out.push({ text: m[2].trim(), line: bodyLine + i });
  });
  return out;
}
