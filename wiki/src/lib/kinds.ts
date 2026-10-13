import { KINDS, type Kind } from "../core/wiki.js";

export const KIND_LABEL: Record<string, string> = Object.fromEntries(KINDS.map((k) => [k.id, k.label]));

/** CSS custom property holding each kind's fill colour. */
export const KIND_VAR: Record<string, string> = {
  guide: "var(--kind-guide)",
  crate: "var(--kind-crate)",
  concept: "var(--kind-concept)",
  algorithm: "var(--kind-algorithm)",
  reference: "var(--kind-reference)",
  tooling: "var(--kind-tooling)",
};

/** Raw colour values, for places that cannot read CSS variables (none at the moment, kept for exports). */
export const KIND_HEX: Record<Kind, string> = {
  guide: "#ffd440",
  crate: "#fe7da8",
  concept: "#27ccf3",
  algorithm: "#a9d877",
  reference: "#bbafe6",
  tooling: "#f8a16f",
};

/** Section directory → kind, and the order sections appear in navigation. */
export const SECTION_ORDER: Kind[] = ["guide", "crate", "concept", "algorithm", "reference", "tooling"];

/**
 * 8×8 pixel icons, one per kind (the Y2K chat avatars). `#` is an ink pixel.
 *   guide: a signpost · crate: a box · concept: a light bulb
 *   algorithm: a gear · reference: a book · tooling: a terminal prompt
 */
export const PIXELS: Record<Kind, string[]> = {
  guide: ["...##...", ".######.", ".#######", "...##...", "#######.", ".######.", "...##...", "..####.."],
  crate: ["########", "#..##..#", "########", "#......#", "#.####.#", "#......#", "#......#", "########"],
  concept: ["..####..", ".#....#.", "#..##..#", "#.#..#.#", ".#....#.", "..#..#..", "..####..", "...##..."],
  algorithm: ["...##...", ".#.##.#.", "..####..", "###..###", "###..###", "..####..", ".#.##.#.", "...##..."],
  reference: [".######.", ".#....##", ".#.##.##", ".#....##", ".#.##.##", ".#....##", ".######.", ".#....#."],
  tooling: ["########", "#......#", "#.#....#", "#..#...#", "#.#....#", "#...##.#", "#......#", "########"],
};
