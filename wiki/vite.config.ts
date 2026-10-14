import { defineConfig } from "vitest/config";
import type { Plugin } from "vite";
import react from "@vitejs/plugin-react";
import { readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = fileURLToPath(new URL("..", import.meta.url));

/** Directories never listed: build output, dependencies, caches, VCS data. */
const SKIP = new Set([".git", "node_modules", "target", "dist", ".venv", "venv", "__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache"]);
const ROOTS = ["crates", "research", "tests", "benchmarks", "examples", "docs", "skills", "wiki"];

function listFiles(dir: string, prefix: string, out: string[]) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (SKIP.has(entry.name)) continue;
    const rel = `${prefix}${entry.name}`;
    if (entry.isDirectory()) listFiles(join(dir, entry.name), `${rel}/`, out);
    else if (entry.isFile()) out.push(rel);
  }
}

/**
 * `virtual:repo-files`: the repository's file list, so the app links inline
 * code like `crates/phasor-exec/src/kernel.rs:144` only when the file exists.
 */
function repoFiles(): Plugin {
  const id = "virtual:repo-files";
  const resolved = `\0${id}`;
  return {
    name: "phasor-repo-files",
    resolveId: (source) => (source === id ? resolved : null),
    load(source) {
      if (source !== resolved) return null;
      const files: string[] = [];
      for (const root of ROOTS) {
        try {
          listFiles(join(repoRoot, root), `${root}/`, files);
        } catch {
          /* a root that does not exist in this checkout */
        }
      }
      files.sort();
      return `export default ${JSON.stringify(files)};`;
    },
  };
}

// The pages live in ./content and the diagrams they embed in ../docs/img, so the
// dev server must be allowed to read the whole repository.
export default defineConfig({
  base: "./",
  plugins: [react(), repoFiles()],
  server: { fs: { allow: [repoRoot] } },
  build: {
    outDir: "dist",
    chunkSizeWarningLimit: 1600,
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.{ts,js}"],
  },
});
