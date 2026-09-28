#!/usr/bin/env bash
# Reads changed paths on stdin and prints which CI areas they touch, as
# `area=true|false` lines for $GITHUB_OUTPUT. `--all` marks every area.
#
#   compiler       the compiler, runtime and tooling suites and the release
#                  builds; also every path not classified below, so a new
#                  directory is tested until someone says otherwise
#   docs_site      the docs site (website/), deployed to GitHub Pages
#   packages_site  the packages website, deployed to Cloudflare
#   registry       the registry service, deployed to Cloudflare
#   editors        the VS Code extension and the Neovim syntax
#
#   git diff --name-only --no-renames <base> HEAD | scripts/ci-changed-areas.sh
set -euo pipefail

compiler=false docs_site=false packages_site=false registry=false editors=false
all() { compiler=true docs_site=true packages_site=true registry=true editors=true; }

if [[ "${1:-}" == --all ]]; then
  all
else
  while IFS= read -r path || [[ -n "$path" ]]; do
    case "$path" in
      # CI itself: run everything once, so the change is exercised.
      .github/* | scripts/ci-changed-areas.sh) all ;;
      # The docs site serves the installers; the release smoke tests them.
      website/docs/public/install.*) docs_site=true compiler=true ;;
      # The extension's packaging check reads the tooling page.
      website/docs/docs/tooling/*) docs_site=true editors=true ;;
      website/*) docs_site=true ;;
      packages-website/*) packages_site=true ;;
      registry/*) registry=true ;;
      # The docs site highlights Mesh with the VS Code grammar.
      tools/editors/vscode-mesh/syntaxes/*) editors=true docs_site=true ;;
      tools/editors/* | scripts/verify-m034-s04-extension.sh) editors=true ;;
      # The docs site shows meshc's version and checks its build with the
      # public-surface contract.
      compiler/meshc/Cargo.toml | scripts/lib/m034_public_surface_contract.py | scripts/lib/repo-identity.json)
        compiler=true docs_site=true ;;
      # Prose nothing builds or tests.
      docs/* | articles/* | README.md | CONTRIBUTING.md | CODE_OF_CONDUCT.md | SECURITY.md | SUPPORT.md | LICENSE) ;;
      # The Rust suites read examples/, tests/, packages/, scripts/ and more.
      *) compiler=true ;;
    esac
  done
fi

printf '%s\n' "compiler=$compiler" "docs_site=$docs_site" "packages_site=$packages_site" \
  "registry=$registry" "editors=$editors"
