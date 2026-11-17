#!/usr/bin/env bash
set -euo pipefail

suite="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$suite/../.." && pwd)"
cd "$root"
out="${PERF_BIN_DIR:-$root/.tmp/performance/bin}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"

# shellcheck source=tools/performance/versions.env
source "$suite/versions.env"
export GOTOOLCHAIN=local

check_version() {
  local component="$1" actual="$2" expected="$3"
  if [[ "$actual" != "$expected" ]]; then
    printf '%s must be %s (found %s)\n' "$component" "$expected" "$actual" >&2
    exit 1
  fi
}

rust_version="$(rustc --version)"
check_version Rust "$(cut -d' ' -f2 <<< "$rust_version")" "$PERF_RUST_VERSION"
check_version Go "$(go env GOVERSION)" "go$PERF_GO_VERSION"
check_version Elixir "$(elixir -e 'IO.write(System.version())')" "$PERF_ELIXIR_VERSION"
otp_version="$(erl +S 1:1 -noshell -eval '
  Path = filename:join([code:root_dir(), "releases", erlang:system_info(otp_release), "OTP_VERSION"]),
  {ok, Version} = file:read_file(Path),
  io:put_chars(Version),
  halt().')"
check_version OTP "$otp_version" "$PERF_OTP_VERSION"
llvm_bin="${LLVM_SYS_211_PREFIX:+$LLVM_SYS_211_PREFIX/bin/}llvm-config"
check_version LLVM "$("$llvm_bin" --version)" "$PERF_LLVM_VERSION"
LLVM_SYS_211_PREFIX="$("$llvm_bin" --prefix)"
export LLVM_SYS_211_PREFIX

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
mkdir -p "$CARGO_TARGET_DIR"
CARGO_TARGET_DIR="$(cd "$CARGO_TARGET_DIR" && pwd)"
cargo build --locked --release -p meshc -p mesh-rt
"$CARGO_TARGET_DIR/release/meshc" build "$suite/mesh" --opt-level 2 --output "$out/mesh"
(
  cd "$suite/go"
  CGO_ENABLED=0 go build -trimpath -o "$out/go" .
)
cargo build --locked --release --manifest-path "$suite/rust/Cargo.toml" \
  --target-dir "$root/.tmp/performance/rust-target"
cp "$root/.tmp/performance/rust-target/release/performance-rust" "$out/rust"
(
  cd "$suite/elixir"
  export MIX_ENV=prod
  mix deps.get --check-locked
  mix release --overwrite
)
release="$suite/elixir/_build/prod/rel/performance_elixir"
printf '#!/usr/bin/env bash\nexec %q start\n' "$release/bin/performance_elixir" > "$out/elixir"
chmod +x "$out/elixir"

{
  printf 'mesh_commit=%s\n' "$(git rev-parse HEAD)"
  if [[ -n "$(git status --porcelain)" ]]; then
    printf 'source_dirty=true\n'
  else
    printf 'source_dirty=false\n'
  fi
  printf 'mesh=%s\n' "$("$CARGO_TARGET_DIR/release/meshc" --version)"
  cat "$suite/versions.env"
} > "$out/versions.txt"
printf 'Built comparison programs in %s\n' "$out"
