import type { Kind } from "../core/wiki.js";
import { KIND_LABEL } from "../lib/kinds";

export function KindTag({ kind }: { kind: Kind }) {
  return (
    <span className="tag" data-kind={kind}>
      {KIND_LABEL[kind] ?? kind}
    </span>
  );
}
