#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp="$(cd "$(mktemp -d)" && pwd -P)"
trap 'rm -rf -- "$tmp"' EXIT

init_checkout() {
  local checkout="$1"
  shift
  mkdir -p "$checkout/compiler/meshc/src" "$checkout/tools/skill"
  git init -q -b main "$@" "$checkout"
  cp "$root/compiler/meshc/build.rs" "$checkout/compiler/meshc/"
  cp -R "$root/tools/skill/mesh" "$checkout/tools/skill/"
  cat >"$checkout/compiler/meshc/Cargo.toml" <<'EOF'
[package]
name = "meshc"
version = "1.2.3"
edition = "2021"
EOF
  cat >"$checkout/compiler/meshc/src/main.rs" <<'EOF'
fn main() {
    let version = match std::env::args().nth(1).as_deref() {
        Some("-V") => env!("CARGO_PKG_VERSION"),
        Some("--version") => env!("MESHC_LONG_VERSION"),
        _ => panic!("expected a version flag"),
    };
    println!("meshc {version}");
}
EOF
  printf '/target/\n' >"$checkout/.gitignore"
  git -C "$checkout" config user.name 'Mesh fixture'
  git -C "$checkout" config user.email fixture@example.invalid
  git -C "$checkout" add .
  git -C "$checkout" commit -qm initial
}

next_commit() {
  git -C "$1" commit-tree 'HEAD^{tree}' -p HEAD -m advance
}

assert_version() {
  local checkout="$1" expected="$2" actual
  (
    cd "$checkout"
    CARGO_TARGET_DIR="$checkout/target" cargo build --offline --quiet --manifest-path compiler/meshc/Cargo.toml
    actual="$(target/debug/meshc --version)"
    if [[ "$actual" != "$expected" ]]; then
      printf '%s\nexpected: %s\nactual:   %s\n' "$checkout" "$expected" "$actual" >&2
      exit 1
    fi
    test "$(target/debug/meshc -V)" = 'meshc 1.2.3'
  )
}

ordinary() {
  local checkout="$tmp/ordinary" archive original updated
  init_checkout "$checkout"
  original="$(git -C "$checkout" rev-parse HEAD)"
  updated="$(next_commit "$checkout")"
  assert_version "$checkout" "meshc 1.2.3 (git $original)"

  git -C "$checkout" update-ref refs/heads/unrelated "$updated"
  git -C "$checkout" update-ref refs/remotes/origin/unrelated "$updated"
  git -C "$checkout" update-ref refs/tags/unrelated "$updated"
  assert_fresh "$checkout"

  git -C "$checkout" pack-refs --all
  assert_version "$checkout" "meshc 1.2.3 (git $original)"
  assert_fresh "$checkout"
  git -C "$checkout" update-ref refs/heads/main "$updated"
  assert_version "$checkout" "meshc 1.2.3 (git $updated)"
  git -C "$checkout" update-ref refs/heads/main "$original"
  git -C "$checkout" pack-refs --all
  assert_version "$checkout" "meshc 1.2.3 (git $original)"
  git -C "$checkout" update-ref refs/heads/main "$updated"
  assert_version "$checkout" "meshc 1.2.3 (git $updated)"
  git -C "$checkout" switch -q --detach "$original"
  assert_version "$checkout" "meshc 1.2.3 (git $original)"

  archive="$checkout/source-archive"
  mkdir "$archive"
  git -C "$checkout" archive HEAD | tar -xf - -C "$archive"
  test "$(git -C "$archive" rev-parse --show-toplevel)" = "$checkout"
  assert_version "$archive" 'meshc 1.2.3'
  (
    cd "$checkout"
    sh "$root/scripts/verify-meshc-source-version.sh" target/debug/meshc
    if sh "$root/scripts/verify-meshc-source-version.sh" "$archive/target/debug/meshc" 2>"$tmp/verify.stderr"; then
      printf 'checkout verifier accepted an archive build\n' >&2
      exit 1
    fi
    grep -Fxq 'actual:   meshc 1.2.3' "$tmp/verify.stderr"
  )
}

assert_fresh() {
  local checkout="$1" log
  log="$(cd "$checkout" && CARGO_TARGET_DIR="$checkout/target" \
    cargo build --color never --offline --verbose --manifest-path compiler/meshc/Cargo.toml 2>&1)"
  if [[ "$log" != *'Fresh meshc v1.2.3'* ]]; then
    printf '%s\nexpected a fresh Cargo build\n' "$log" >&2
    exit 1
  fi
}

packed_slash_branch() {
  local checkout="$tmp/packed-slash" branch=feature/nested/version original updated
  init_checkout "$checkout"
  git -C "$checkout" switch -qc "$branch"
  original="$(git -C "$checkout" rev-parse HEAD)"
  updated="$(next_commit "$checkout")"
  assert_version "$checkout" "meshc 1.2.3 (git $original)"

  git -C "$checkout" pack-refs --all
  test ! -d "$checkout/.git/refs/heads/feature"
  assert_version "$checkout" "meshc 1.2.3 (git $original)"
  assert_fresh "$checkout"

  git -C "$checkout" update-ref "refs/heads/$branch" "$updated"
  assert_version "$checkout" "meshc 1.2.3 (git $updated)"
}

path_whitespace() {
  local checkout="$tmp/whitespace" original updated
  init_checkout "$checkout" --separate-git-dir "$tmp/git metadata "
  original="$(git -C "$checkout" rev-parse HEAD)"
  updated="$(next_commit "$checkout")"
  assert_version "$checkout" "meshc 1.2.3 (git $original)"
  assert_fresh "$checkout"
  git -C "$checkout" update-ref refs/heads/main "$updated"
  assert_version "$checkout" "meshc 1.2.3 (git $updated)"
}

files_linked() {
  local common="$tmp/files" linked="$tmp/files-linked" original updated
  init_checkout "$common"
  original="$(git -C "$common" rev-parse HEAD)"
  updated="$(next_commit "$common")"
  git -C "$common" worktree add -q --detach "$linked" "$original"
  git -C "$linked" symbolic-ref HEAD refs/worktree/meshc-source
  git -C "$common" update-ref worktrees/files-linked/refs/worktree/meshc-source "$original"
  assert_version "$linked" "meshc 1.2.3 (git $original)"
  git -C "$common" update-ref worktrees/files-linked/refs/worktree/meshc-source "$updated"
  test "$(git -C "$linked" rev-parse HEAD)" = "$updated"
  assert_version "$linked" "meshc 1.2.3 (git $updated)"
}

reftable_linked() {
  local common="$tmp/reftable" linked="$tmp/reftable-linked" original updated
  init_checkout "$common" --ref-format=reftable
  original="$(git -C "$common" rev-parse HEAD)"
  updated="$(next_commit "$common")"
  git -C "$common" worktree add -q -b fixture-linked "$linked" "$original"
  assert_version "$linked" "meshc 1.2.3 (git $original)"
  git -C "$common" update-ref refs/heads/fixture-linked "$updated"
  test "$(git -C "$linked" rev-parse HEAD)" = "$updated"
  assert_version "$linked" "meshc 1.2.3 (git $updated)"
  git -C "$linked" switch -q --detach "$original"
  assert_version "$linked" "meshc 1.2.3 (git $original)"
}

legacy_layout() {
  local checkout="$tmp/legacy" shim="$tmp/bin" real_git
  init_checkout "$checkout"
  real_git="$(command -v git)"
  mkdir "$shim"
  # Git 2.30 echoes the unknown flag and leaves --git-common-dir relative.
  cat >"$shim/git" <<'EOF'
#!/bin/sh
if [ "$1" = rev-parse ] && [ "${2:-}" = --show-prefix ] && [ "${3:-}" = --path-format=absolute ]; then
  printf '\n--path-format=absolute\n'
  shift 3
  exec "$REAL_GIT" rev-parse "$@"
fi
exec "$REAL_GIT" "$@"
EOF
  chmod +x "$shim/git"
  PATH="$shim:$PATH" REAL_GIT="$real_git" assert_version "$checkout" 'meshc 1.2.3'
}

legacy_symbolic_ref() {
  local checkout="$tmp/legacy-symbolic-ref" shim="$tmp/symbolic-ref-bin" real_git
  init_checkout "$checkout"
  real_git="$(command -v git)"
  mkdir "$shim"
  # Git 2.34 rejects --no-recurse with a usage error.
  cat >"$shim/git" <<'EOF'
#!/bin/sh
if [ "$1" = symbolic-ref ] && [ "${3:-}" = --no-recurse ]; then
  printf "error: unknown option 'no-recurse'\n" >&2
  exit 129
fi
exec "$REAL_GIT" "$@"
EOF
  chmod +x "$shim/git"
  PATH="$shim:$PATH" REAL_GIT="$real_git" assert_version "$checkout" 'meshc 1.2.3'
}

if [[ $# -eq 0 ]]; then
  set -- ordinary packed_slash_branch path_whitespace files_linked reftable_linked legacy_layout legacy_symbolic_ref
fi
for fixture in "$@"; do
  if [[ "$fixture" == reftable_linked && "$(git init -h 2>&1 || true)" != *--ref-format* ]]; then
    printf 'SKIP reftable fixture (Git lacks --ref-format)\n'
    continue
  fi
  case "$fixture" in
    ordinary|packed_slash_branch|path_whitespace|files_linked|reftable_linked|legacy_layout|legacy_symbolic_ref) "$fixture" ;;
    *) printf 'Unknown fixture: %s\n' "$fixture" >&2; exit 2 ;;
  esac
  printf 'PASS %s\n' "$fixture"
done
