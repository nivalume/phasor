import files from "virtual:repo-files";

const fileSet = new Set<string>(files);
const dirSet = new Set<string>();
for (const f of files) {
  for (let i = f.indexOf("/"); i >= 0; i = f.indexOf("/", i + 1)) dirSet.add(f.slice(0, i));
}

/** Whether a repo-relative path is a file or directory of this checkout (at build time). */
export function repoExists(path: string): boolean {
  return fileSet.has(path) || dirSet.has(path);
}
