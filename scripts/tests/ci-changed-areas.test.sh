#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly SCRIPT_PATH="${ROOT_DIR}/scripts/ci-changed-areas.sh"

# expect "<changed paths>" "<areas that must be true, space separated>"
expect() {
  local actual expected=""
  actual="$(printf '%s' "$1" | bash "${SCRIPT_PATH}" | sed -n 's/=true$//p' | tr '\n' ' ')"
  [[ -n "$2" ]] && expected="$2 "
  [[ "${actual}" == "${expected}" ]] || {
    printf 'contract failure: %q -> [%s], expected [%s]\n' "$1" "${actual}" "${expected}" >&2
    exit 1
  }
}

expect $'website/docs/index.md\nwebsite/docs/.vitepress/theme/components/landing/Hero.vue' "docs_site"
expect "compiler/mesh-typeck/src/infer.rs" "compiler"
expect "somewhere/new.txt" "compiler"
expect "registry/src/main.rs" "registry"
expect "packages-website/src/app.html" "packages_site"
expect "tools/editors/vscode-mesh/syntaxes/mesh.tmLanguage.json" "docs_site editors"
expect "website/docs/public/install.sh" "compiler docs_site"
expect $'README.md\ndocs/native-packages.md' ""
expect "" ""
expect ".github/workflows/deploy.yml" "compiler docs_site packages_site registry editors"
[[ "$(bash "${SCRIPT_PATH}" --all </dev/null | grep -c '=true$')" == 5 ]] || {
  echo 'contract failure: --all did not mark every area' >&2
  exit 1
}

printf 'ci changed areas contract passed\n'
