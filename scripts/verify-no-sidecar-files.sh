#!/usr/bin/env bash
# Refuse commits that carry filesystem droppings instead of source.
#
# A checkout on a drive without Unix metadata (exFAT, FAT, some network shares)
# does two things behind git's back. macOS keeps each file's extended attributes
# in a `._name` AppleDouble sidecar next to it, and every file reads as
# executable. One `git add -A` there once committed 3,932 sidecars and flipped
# 3,241 files from 100644 to 100755.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  bash scripts/verify-no-sidecar-files.sh --staged
  bash scripts/verify-no-sidecar-files.sh --diff-range <git-range>
EOF
}

case "${1:-}" in
  --staged)
    [[ $# -eq 1 ]] || { usage >&2; exit 1; }
    diff_args=(--cached)
    ;;
  --diff-range)
    [[ $# -eq 2 ]] || { usage >&2; exit 1; }
    diff_args=("$2")
    ;;
  *)
    usage >&2
    exit 1
    ;;
esac

cd "$(git rev-parse --show-toplevel)"
status=0

# AppleDouble sidecars and Finder metadata, added or modified.
sidecars="$(git diff "${diff_args[@]}" --name-only --diff-filter=ACMR |
  grep -E '(^|/)(\._[^/]*|\.DS_Store)$' || true)"
if [[ -n "$sidecars" ]]; then
  echo "verify-no-sidecar-files: macOS metadata files must not be committed:" >&2
  sed -n '1,20s/^/  - /p' <<<"$sidecars" >&2
  echo "  unstage them with: git rm -r --cached --ignore-unmatch -- ':(glob)**/._*' ':(glob)**/.DS_Store'" >&2
  status=1
fi

# A drive that cannot store the executable bit makes every file look changed
# to 100755. One script gaining the bit is normal; dozens at once is that.
flipped="$(git diff "${diff_args[@]}" --raw --diff-filter=M |
  awk '$1 == ":100644" && $2 == "100755"' | wc -l | tr -d ' ')"
if [[ "$flipped" -gt 25 ]]; then
  echo "verify-no-sidecar-files: $flipped files change mode 100644 -> 100755 and nothing else." >&2
  echo "  This filesystem does not keep the executable bit. Run:" >&2
  echo "    git config core.fileMode false && git update-index --really-refresh" >&2
  status=1
fi

exit "$status"
