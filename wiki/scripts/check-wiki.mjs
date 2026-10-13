#!/usr/bin/env node
// Validate the wiki content: frontmatter, ids, links, anchors, images, source
// paths and reachability. Exits non-zero on any error.
//
//   node scripts/check-wiki.mjs                         # everything
//   node scripts/check-wiki.mjs --files a.md b.md       # report only for these files
//   node scripts/check-wiki.mjs --registry ids.json     # treat planned ids as existing (while pages are being written)
//   node scripts/check-wiki.mjs --json                  # machine-readable report
//
// Run from the wiki/ directory or anywhere; paths are resolved from this file.

import { readFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { join, relative, resolve, dirname as pathDirname } from "node:path";
import { fileURLToPath } from "node:url";
import {
  buildWiki,
  KIND_IDS,
  CONTENT_ROOT,
  parseSourceRef,
  extractInlineCode,
  looksLikeRepoPath,
  resolveCodePath,
} from "../src/core/wiki.js";

const here = pathDirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "../..");
const contentDir = join(repoRoot, CONTENT_ROOT);

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const listAfter = (name) => {
  const i = args.indexOf(name);
  if (i < 0) return null;
  const out = [];
  for (let j = i + 1; j < args.length && !args[j].startsWith("--"); j++) out.push(args[j]);
  return out;
};

function walk(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) out.push(...walk(full));
    else if (name.endsWith(".md")) out.push(full);
  }
  return out;
}

const files = walk(contentDir).map((full) => ({
  path: relative(repoRoot, full).split("\\").join("/"),
  text: readFileSync(full, "utf8"),
}));
const wiki = buildWiki(files);
const problems = [...wiki.problems];
const add = (level, path, line, message) => problems.push({ level, path, line, message });

const registryPath = listAfter("--registry")?.[0];
/** @type {Map<string, {kind: string, headings?: string[]}>} */
const planned = new Map();
if (registryPath) {
  for (const entry of JSON.parse(readFileSync(resolve(registryPath), "utf8"))) planned.set(entry.id, entry);
}
const pageExists = (id) => wiki.pages.has(id) || planned.has(id);
const kindOf = (id) => wiki.pages.get(id)?.kind ?? planned.get(id)?.kind;
const repoExists = (p) => existsSync(join(repoRoot, p));

for (const page of wiki.pages.values()) {
  const at = (line, level, msg) => add(level, page.path, line, msg);
  const d = page.data;
  if (typeof d.title !== "string" || !d.title.trim()) at(1, "error", "frontmatter: `title` is required");
  if (!KIND_IDS.includes(page.kind)) at(1, "error", `frontmatter: \`kind\` must be one of ${KIND_IDS.join(", ")} (got "${d.kind ?? ""}")`);
  if (typeof d.summary !== "string" || !d.summary.trim()) at(1, "error", "frontmatter: `summary` is required (one sentence)");
  else if (d.summary.length > 160) at(1, "warning", `frontmatter: \`summary\` is ${d.summary.length} characters; keep it under 160`);
  if (page.id !== "index" && page.section === "") at(1, "error", "pages other than index.md must live in a section directory");
  const sectionKind = { guides: "guide", crates: "crate", concepts: "concept", algorithms: "algorithm", reference: "reference", tooling: "tooling" }[page.section];
  if (sectionKind && sectionKind !== page.kind) at(1, "error", `kind "${page.kind}" does not match its directory ${page.section}/ (expected "${sectionKind}")`);
  const known = new Set(["title", "kind", "summary", "parent", "depends", "related", "tags", "sources", "order"]);
  for (const k of Object.keys(d)) if (!known.has(k)) at(1, "warning", `frontmatter: unknown key \`${k}\``);

  if (page.kind === "concept" || page.kind === "algorithm") {
    if (!page.parent) at(1, "error", "concept and algorithm pages need `parent:` (the crate or tooling page they belong to)");
  }
  if (page.parent) {
    if (!pageExists(page.parent)) at(1, "error", `parent "${page.parent}" is not a page`);
    else if (!["crate", "tooling"].includes(kindOf(page.parent) ?? "")) at(1, "error", `parent "${page.parent}" must be a crate or tooling page`);
  }
  for (const dep of page.depends) {
    if (page.kind !== "crate") at(1, "error", "`depends` is only for crate pages");
    if (!pageExists(dep)) at(1, "error", `depends: "${dep}" is not a page`);
    else if (kindOf(dep) !== "crate") at(1, "error", `depends: "${dep}" is not a crate page`);
  }
  for (const r of page.related) if (!pageExists(r)) at(1, "error", `related: "${r}" is not a page`);
  for (const s of page.sources) {
    const { path } = parseSourceRef(s);
    if (!repoExists(path)) at(1, "error", `sources: ${path} does not exist in the repository`);
  }

  for (const link of page.links) {
    const t = link.target;
    if (t.type === "invalid") at(link.line, "error", `link ${link.href}: ${t.reason}`);
    else if (t.type === "page") {
      if (!pageExists(t.id)) at(link.line, "error", `link ${link.href}: no page "${t.id}"`);
      else if (t.anchor && wiki.pages.has(t.id)) {
        const target = wiki.pages.get(t.id);
        if (!target.headings.some((h) => h.slug === t.anchor)) at(link.line, "error", `link ${link.href}: no heading #${t.anchor} in ${t.id}`);
      }
      if (link.href.replace(/#.*/, "").endsWith("/")) at(link.line, "error", `link ${link.href}: link to the .md file`);
    } else if (t.type === "repo") {
      if (!repoExists(t.path)) at(link.line, "error", `link ${link.href}: ${t.path} does not exist in the repository`);
    } else if (t.type === "anchor") {
      if (!page.headings.some((h) => h.slug === t.anchor)) at(link.line, "error", `link ${link.href}: no heading #${t.anchor} in this page`);
    }
  }

  // Inline code written as a repository path is linked by the app only when the file exists.
  for (const code of extractInlineCode(page.body, page.bodyLine)) {
    if (looksLikeRepoPath(code.text) && !resolveCodePath(code.text, page, repoExists)) {
      at(code.line, "warning", `inline path \`${code.text}\` does not exist in the repository, so it is shown without a link`);
    }
  }
}

// Every page except index must be reachable: at least one other page links to it.
if (!registryPath) {
  if (!wiki.pages.has("index")) add("error", `${CONTENT_ROOT}/index.md`, 1, "index.md is missing");
  for (const page of wiki.pages.values()) {
    if (page.id === "index") continue;
    const inbound = (wiki.incoming.get(page.id) ?? []).filter((e) => e.kind === "link");
    if (inbound.length === 0) add("error", page.path, 1, "orphan: no other page links to this page");
  }
}

const only = listAfter("--files");
const onlySet = only ? new Set(only.map((f) => relative(repoRoot, resolve(f)).split("\\").join("/"))) : null;
const shown = problems.filter((p) => !onlySet || onlySet.has(p.path));
shown.sort((a, b) => a.path.localeCompare(b.path) || a.line - b.line);

if (flag("--json")) {
  console.log(JSON.stringify({ pages: wiki.pages.size, edges: wiki.edges.length, problems: shown }, null, 2));
} else {
  for (const p of shown) console.log(`${p.level === "error" ? "ERROR" : "warn "} ${p.path}:${p.line}  ${p.message}`);
  const errors = shown.filter((p) => p.level === "error").length;
  const warnings = shown.length - errors;
  const byKind = {};
  for (const e of wiki.edges) byKind[e.kind] = (byKind[e.kind] ?? 0) + 1;
  console.log(
    `${wiki.pages.size} pages, ${wiki.edges.length} edges (${Object.entries(byKind).map(([k, v]) => `${k} ${v}`).join(", ")}); ` +
      `${errors} error(s), ${warnings} warning(s)`,
  );
}
process.exit(shown.some((p) => p.level === "error") ? 1 : 0);
