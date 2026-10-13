import { CODE_REF, CONTENT_REF, REPO_URL } from "../config";
import { parseSourceRef } from "../core/wiki.js";

const CODE_PREFIXES = ["crates/", "research/", "tests/", "benchmarks/", "examples/"];

/** GitHub URL for a repo path, with an optional `:line` or `:a-b` suffix. */
export function githubUrl(ref: string): string {
  const { path, start, end } = parseSourceRef(ref);
  const isCode = CODE_PREFIXES.some((p) => path.startsWith(p));
  const commit = isCode ? CODE_REF : CONTENT_REF;
  const kind = path.endsWith("/") ? "tree" : "blob";
  let url = `${REPO_URL}/${kind}/${commit}/${path}`;
  if (start != null) url += end != null ? `#L${start}-L${end}` : `#L${start}`;
  return url;
}

export function editUrl(path: string): string {
  return `${REPO_URL}/edit/${CONTENT_REF}/${path}`;
}
