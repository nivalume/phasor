#!/usr/bin/env bash
# Link (or copy) every skill under skills/ into the user-level skill directories
# of Claude Code, Codex and Grok Build, or into this repository's own
# .claude/.agents/.grok directories.
#
#   ./skills/install.sh                 # symlink into ~/.claude/skills ~/.agents/skills ~/.grok/skills
#   ./skills/install.sh --copy          # copy instead of symlink
#   ./skills/install.sh --project       # (re)create the in-repo links instead of user-level ones
#   ./skills/install.sh --uninstall     # remove what this script installed (user-level by default)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/.." && pwd)"
mode=link
scope=user
for arg in "$@"; do
  case "$arg" in
    --copy) mode=copy ;;
    --uninstall) mode=uninstall ;;
    --project) scope=project ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

if [ "$scope" = user ]; then
  targets=("$HOME/.claude/skills" "$HOME/.agents/skills" "$HOME/.grok/skills")
else
  targets=("$repo/.claude/skills" "$repo/.agents/skills" "$repo/.grok/skills")
fi

for skill in "$here"/*/; do
  name="$(basename "$skill")"
  [ -f "$skill/SKILL.md" ] || continue
  for dir in "${targets[@]}"; do
    dest="$dir/$name"
    case "$mode" in
      uninstall)
        if [ -L "$dest" ] || [ -d "$dest" ]; then rm -rf "$dest"; echo "removed $dest"; fi ;;
      copy)
        mkdir -p "$dir"; rm -rf "$dest"; cp -R "$skill" "$dest"; echo "copied  $dest" ;;
      link)
        mkdir -p "$dir"
        if [ "$scope" = project ]; then rel="../../skills/$name"; else rel="$skill"; fi
        rm -rf "$dest"; ln -s "${rel%/}" "$dest"; echo "linked  $dest -> ${rel%/}" ;;
    esac
  done
done
