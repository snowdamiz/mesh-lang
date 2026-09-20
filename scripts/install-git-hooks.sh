#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if ! git rev-parse --show-toplevel >/dev/null 2>&1; then
  echo "install-git-hooks: must run inside a git worktree" >&2
  exit 1
fi

chmod +x .githooks/pre-commit scripts/verify-whitespace.sh scripts/verify-no-sidecar-files.sh

git config core.hooksPath .githooks

# A drive that cannot store the executable bit (exFAT, FAT) reports every file
# as 100755, and with core.fileMode on git records that as a change to each one.
probe="$(mktemp "$ROOT_DIR/.git/filemode-probe.XXXXXX")"
chmod 644 "$probe"
if [[ -x "$probe" ]]; then
  git config core.fileMode false
  echo "install-git-hooks: this filesystem does not keep the executable bit; set core.fileMode=false"
fi
rm -f "$probe"

echo "install-git-hooks: configured core.hooksPath=.githooks"
echo "install-git-hooks: pre-commit now runs scripts/verify-no-sidecar-files.sh --staged"
echo "install-git-hooks: pre-commit now runs scripts/verify-whitespace.sh --staged --fix"
