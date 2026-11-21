#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 1 ]]; then
  echo "Usage: $0 /path/to/pinned/release" >&2
  exit 2
fi
release="$(cd "$1" && pwd)"
cd "$(dirname "${BASH_SOURCE[0]}")/.."
runbook=website/docs/docs/concurrency/jobs-operations.md
reference=website/docs/docs/concurrency/index.md

toolchain_proof="$(python3 - "$release" <<'PY'
import hashlib
import json
from pathlib import Path
import re
import sys

release = Path(sys.argv[1])
pin = json.loads((release / "jobs-parity-toolchain.json").read_text())
revision = pin["mesh_sha"]
if not re.fullmatch("[0-9a-f]{40}", revision) or pin["profile"] != "release":
    raise SystemExit("Unexpected Jobs release toolchain")
print(revision)
for name in ("meshc", "libmesh_rt.a"):
    with (release / name).open("rb") as stream:
        digest = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    if digest.hexdigest() != pin["files_sha256"][name]:
        raise SystemExit(f"Pinned digest mismatch for {name}")
    print(f"{digest.hexdigest()}  {name}")
PY
)"
runtime="${toolchain_proof%%$'\n'*}"
printf 'J7 lane 9 | %s\n' "$(uname -smr)"
printf 'Checkout source: %s\n' "$(git rev-parse HEAD)"
printf 'Compiler/runtime source: %s\n' "$runtime"
git diff --exit-code "$runtime" -- compiler Cargo.toml Cargo.lock .cargo \
  ':(exclude,glob)compiler/**/tests/**'
printf 'Production compiler and runtime source and Cargo inputs match.\n'
shasum -a 256 scripts/verify-jobs-guide.sh "$reference" "$runbook"
printf '%s\n' "${toolchain_proof#*$'\n'}"

pg_bin="${PG_BIN_DIR:-$(pg_config --bindir)}"
export PATH="$release:$pg_bin:$PATH"
export MESH_RT_LIB_PATH="$release/libmesh_rt.a"
export MESH_SCHEDULER_MIN_WORKERS=1 MESH_SCHEDULER_MAX_WORKERS=1
unset HOLD_JOB PGSERVICE PGHOSTADDR PGOPTIONS
meshc --version

mkdir -p .tmp
scratch="$(mktemp -d "$PWD/.tmp/j7-lane-9.XXXXXX")"
data="$scratch/data"
cleanup() {
  if pg_ctl -D "$data" status >/dev/null 2>&1; then
    if ! pg_ctl -D "$data" -m fast -t 10 stop >/dev/null &&
       ! pg_ctl -D "$data" -m immediate -t 10 stop >/dev/null; then
      printf 'PostgreSQL is still running. Preserved %s\n' "$scratch" >&2
      exit 1
    fi
  fi
  rm -rf -- "$scratch"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
export TMPDIR="$scratch"
password="$(python3 -c 'import secrets; print(secrets.token_hex(16))')"
(umask 077; printf '%s\n' "$password" > "$scratch/password")
initdb -D "$data" -U jobs_demo --no-locale --encoding=UTF8 \
  --auth=scram-sha-256 --pwfile="$scratch/password" > "$scratch/initdb.log"
rm -- "$scratch/password"
port="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
pg_ctl -D "$data" -l "$scratch/postgres.log" \
  -o "-h 127.0.0.1 -k '' -p $port -c shared_buffers=16MB -c max_connections=12" -t 10 -w start
export DATABASE_URL="postgres://jobs_demo:$password@127.0.0.1:$port/postgres?sslmode=disable"
unset password
printf 'PostgreSQL server: %s (loopback SCRAM)\n' \
  "$(psql -XqAtw -v ON_ERROR_STOP=1 "$DATABASE_URL" -c 'SHOW server_version')"
psql -Xqw -v ON_ERROR_STOP=1 "$DATABASE_URL" -c 'CREATE DATABASE jobs_demo'
DATABASE_URL="${DATABASE_URL%/postgres\?*}/jobs_demo?sslmode=disable"

python3 - "$scratch" "$reference" "$runbook" <<'PY'
from pathlib import Path
import re
import sys

scratch, reference, runbook = map(Path, sys.argv[1:])
section = runbook.read_text().split("## Try enqueue, retry, drain, and lease recovery\n", 1)[1]
section = section.split("\n## ", 1)[0]
setup, app, *commands = re.findall(r"```(?:bash|mesh)\n(.*?)```", section, re.S)
(scratch / "walkthrough.sh").write_text(
    setup + "cat > main.mpl <<'MESH_APP'\n" + app + "MESH_APP\n" + "\n".join(commands)
)
section = reference.read_text().split("## Durable jobs\n", 1)[1]
section = section.split("\n### Recurring jobs", 1)[0]
enqueue, worker, drain = re.findall(r"```mesh\n(.*?)```", section, re.S)
(scratch / "reference").mkdir()
(scratch / "reference/main.mpl").write_text("\n".join((enqueue, worker, drain)))
PY

printf '\nRun the Jobs operations walkthrough from its Markdown commands.\n'
(cd "$scratch" && bash walkthrough.sh)

printf '\nRun the concurrency reference snippets with a fresh order.\n'
psql -Xqw -v ON_ERROR_STOP=1 "$DATABASE_URL" -c 'CREATE TABLE orders (id bigint PRIMARY KEY)'
cat >> "$scratch/reference/main.mpl" <<'MESH_APP'

fn run(pool :: PoolHandle) -> String!String do
  let order = submit_order(pool)?
  println("Submitted order #{order}")
  let report = drain_receipts(pool,
    """{"namespace":"shop","queues":{"default":1},"workers":[{"worker":"send-receipt","version":1}]}""")?
  let runner = start_receipts(pool)?
  let _ = Jobs.stop(runner, 15000)?
  let _ = Jobs.join(runner)?
  Ok(report)
end

fn main() do
  case Pool.open(Env.get("DATABASE_URL", ""), 1, 2, 5000) do
    Ok(pool) ->
      let result = run(pool)
      Pool.close(pool)
      case result do
        Ok(report) -> println(report)
        Err(error) -> panic(error)
      end
    Err(error) -> panic(error)
  end
end
MESH_APP
(cd "$scratch/reference" && meshc build . --output snippets && ./snippets) | tee "$scratch/snippets.out"
tail -n 1 "$scratch/snippets.out" | jq -e '.claimed == 1 and .outcomes.completed == 1'
order="$(psql -XqAtw -v ON_ERROR_STOP=1 "$DATABASE_URL" -c \
  "SELECT orders.id, jobs.state, jobs.attempt FROM orders JOIN mesh_jobs.jobs AS jobs
   ON jobs.args->>'order_id' = orders.id::text WHERE jobs.namespace = 'shop'")"
test "$order" = "42|completed|1"
printf 'Committed order and job: %s\n' "$order"
cleanup
trap - EXIT
printf 'Disposable PostgreSQL cluster removed.\n'
printf '\nJ7 lane 9 PASS\n'
