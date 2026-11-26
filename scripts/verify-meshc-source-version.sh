#!/bin/sh
set -eu

meshc=$1
if [ ! -e .git ] || [ -n "$(git rev-parse --show-prefix)" ]; then
  printf 'Run from the Mesh Git checkout root.\n' >&2
  exit 1
fi
revision=$(git rev-parse --verify 'HEAD^{commit}')
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' compiler/meshc/Cargo.toml)
test -n "$version"
short=$("$meshc" -V)
long=$("$meshc" --version)
expected_short="meshc $version"
expected_long="$expected_short (git $revision)"

if [ "$short" != "$expected_short" ] || [ "$long" != "$expected_long" ]; then
  printf 'expected: %s\nactual:   %s\n' "$expected_short" "$short" "$expected_long" "$long" >&2
  exit 1
fi
printf '%s\n' "$long"
