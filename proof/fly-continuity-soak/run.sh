#!/usr/bin/env bash
set -euo pipefail

mkdir -p /data/tmp /data/evidence
export TMPDIR=/data/tmp

set +e
/usr/local/bin/meshc proof continuity-soak \
  --evidence-dir /data/evidence \
  2>&1 | tee /data/evidence/soak.log
soak_status=${PIPESTATUS[0]}
set -e

printf '%s\n' "${soak_status}" > /data/evidence/exit-code
date -u +%Y-%m-%dT%H:%M:%SZ > /data/evidence/finished-at
if [[ ${soak_status} -eq 0 ]]; then
  touch /data/evidence/release-pass
else
  touch /data/evidence/release-fail
fi

# Keep the volume reachable long enough for the retained evidence to be copied
# off the Machine. The monitor stops the Machine after retrieval.
exec sleep infinity
