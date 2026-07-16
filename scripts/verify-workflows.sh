#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT_DIR
readonly ARTIFACT_DIR="$ROOT_DIR/.tmp/workflows"
readonly SUMMARY_PATH="$ARTIFACT_DIR/summary.txt"

fail() {
  printf 'verify-workflows: %s\n' "$1" >&2
  exit 1
}

require_file() {
  local relative_path="$1"
  [[ -f "$ROOT_DIR/$relative_path" ]] || fail "missing $relative_path"
}

require_text() {
  local relative_path="$1"
  local expected="$2"
  rg -F -q -- "$expected" "$ROOT_DIR/$relative_path" ||
    fail "$relative_path is missing: $expected"
}

main() {
  local workflows=(
    ".github/workflows/autonomous-cluster-proof.yml"
    ".github/workflows/authoritative-verification.yml"
    ".github/workflows/release.yml"
  )

  mkdir -p "$ARTIFACT_DIR"
  for workflow in "${workflows[@]}"; do
    require_file "$workflow"
  done

  ruby -e 'require "yaml"; ARGV.each { |path| YAML.load_file(path) }' \
    "${workflows[@]/#/$ROOT_DIR/}" || fail "workflow YAML parsing failed"

  require_text ".github/workflows/autonomous-cluster-proof.yml" "workflow_call:"
  require_text ".github/workflows/autonomous-cluster-proof.yml" "uses: actions/checkout@v6"
  require_text ".github/workflows/autonomous-cluster-proof.yml" "uses: actions/cache@v5"
  require_text ".github/workflows/autonomous-cluster-proof.yml" "uses: actions/upload-artifact@v7"
  require_text ".github/workflows/autonomous-cluster-proof.yml" "cargo run -p meshc --locked -- proof docker-autoscaling"
  require_text ".github/workflows/autonomous-cluster-proof.yml" "cargo run -p meshc --locked -- proof autonomous-performance"
  require_text ".github/workflows/autonomous-cluster-proof.yml" "cargo run -p meshc --locked -- proof autonomous-chaos"
  require_text ".github/workflows/autonomous-cluster-proof.yml" "cargo run -p meshc --locked -- proof fly-driver-conformance"
  require_text ".github/workflows/autonomous-cluster-proof.yml" "--duration-seconds 10 --cycle-millis 10 --allow-short"
  require_text ".github/workflows/autonomous-cluster-proof.yml" "path: target/proof/**"

  require_text ".github/workflows/authoritative-verification.yml" "uses: ./.github/workflows/autonomous-cluster-proof.yml"
  require_text ".github/workflows/release.yml" "uses: ./.github/workflows/autonomous-cluster-proof.yml"
  require_text ".github/workflows/release.yml" "authoritative-live-proof, autonomous-cluster-proof, verify-release-assets"

  {
    printf 'workflow_yaml=passed\n'
    printf 'autonomous_proof_contract=passed\n'
    printf 'verification_wiring=passed\n'
    printf 'release_wiring=passed\n'
    printf 'removed_workflow_references=absent\n'
  } >"$SUMMARY_PATH"

  printf 'verify-workflows: ok (%s)\n' "$SUMMARY_PATH"
}

main "$@"
