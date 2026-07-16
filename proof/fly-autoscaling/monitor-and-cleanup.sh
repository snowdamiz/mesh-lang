#!/usr/bin/env bash
set -euo pipefail

readonly script_name=${0##*/}
state_dir=${1:-}
[[ -n "${state_dir}" ]] || { printf 'Usage: %s STATE_DIR\n' "${script_name}" >&2; exit 2; }
readonly state_file="${state_dir}/state.json"
[[ -f "${state_file}" ]] || { printf 'state file not found: %s\n' "${state_file}" >&2; exit 2; }

for command in flyctl jq; do
  command -v "${command}" >/dev/null || { printf 'required command missing: %s\n' "${command}" >&2; exit 2; }
done

run_id=$(jq -r '.run_id' "${state_file}")
readonly run_id
controller_app=$(jq -r '.controller_app' "${state_file}")
readonly controller_app
data_app=$(jq -r '.data_app' "${state_file}")
readonly data_app
runner_app=$(jq -r '.runner_app' "${state_file}")
readonly runner_app
mpg_id=$(jq -r '.mpg_id' "${state_file}")
readonly mpg_id
mpg_name=$(jq -r '.mpg_name' "${state_file}")
readonly mpg_name
runner_machine=$(jq -r '.runner_machine' "${state_file}")
readonly runner_machine
runner_volume=$(jq -r '.runner_volume' "${state_file}")
readonly runner_volume
material_file=$(jq -r '.material_file' "${state_file}")
readonly material_file

[[ "${run_id}" =~ ^[0-9]{12}$ ]] || { printf 'invalid proof run ID\n' >&2; exit 2; }
[[ "${controller_app}" == "mesh-fly-c-${run_id}" ]] || { printf 'controller app fence mismatch\n' >&2; exit 2; }
[[ "${data_app}" == "mesh-fly-d-${run_id}" ]] || { printf 'data app fence mismatch\n' >&2; exit 2; }
[[ "${runner_app}" == "mesh-fly-r-${run_id}" ]] || { printf 'runner app fence mismatch\n' >&2; exit 2; }
[[ "${mpg_name}" == "mesh-fly-pg-${run_id}" ]] || { printf 'Postgres fence mismatch\n' >&2; exit 2; }

mkdir -p "${state_dir}/monitor"
observed_at=$(date -u +%Y%m%dT%H%M%SZ)
readonly observed_at
for app in "${controller_app}" "${data_app}" "${runner_app}"; do
  flyctl machine list --app "${app}" --json > "${state_dir}/monitor/${observed_at}-${app}.json" 2>/dev/null \
    || printf '[]\n' > "${state_dir}/monitor/${observed_at}-${app}.json"
done
flyctl mpg status "${mpg_id}" --json 2>/dev/null | jq 'del(.credentials)' \
  > "${state_dir}/monitor/${observed_at}-postgres.json" \
  || printf '{}\n' > "${state_dir}/monitor/${observed_at}-postgres.json"

finished_at=$(flyctl ssh console --app "${runner_app}" --machine "${runner_machine}" --quiet \
  --command 'cat /data/evidence/finished-at 2>/dev/null' 2>/dev/null || true)
last_checkpoint=$(flyctl ssh console --app "${runner_app}" --machine "${runner_machine}" --quiet \
  --command 'tail -n 1 /data/evidence/checkpoints.jsonl 2>/dev/null' 2>/dev/null || true)

if [[ -z "${finished_at}" ]]; then
  jq -n --arg run_id "${run_id}" --arg observed_at "${observed_at}" \
    --argjson checkpoint "${last_checkpoint:-null}" \
    '{terminal:false,run_id:$run_id,observed_at:$observed_at,last_checkpoint:$checkpoint}' \
    | tee "${state_dir}/monitor/latest.json"
  exit 0
fi

readonly retrieved_dir="${state_dir}/retrieved"
rm -rf "${retrieved_dir}"
mkdir -p "${retrieved_dir}"
flyctl ssh sftp get /data/evidence "${retrieved_dir}" --recursive \
  --app "${runner_app}" --machine "${runner_machine}" --quiet

# Capture the final provider state before deleting anything.
for app in "${controller_app}" "${data_app}" "${runner_app}"; do
  flyctl machine list --app "${app}" --json > "${state_dir}/final-${app}.json" 2>/dev/null \
    || printf '[]\n' > "${state_dir}/final-${app}.json"
done
flyctl mpg status "${mpg_id}" --json 2>/dev/null | jq 'del(.credentials)' \
  > "${state_dir}/final-postgres.json" \
  || printf '{}\n' > "${state_dir}/final-postgres.json"

flyctl machine destroy --app "${runner_app}" --force "${runner_machine}" >/dev/null 2>&1 || true
flyctl volumes destroy --app "${runner_app}" --yes "${runner_volume}" >/dev/null 2>&1 || true
flyctl apps destroy "${controller_app}" "${data_app}" "${runner_app}" --yes >/dev/null
flyctl mpg destroy "${mpg_id}" --yes >/dev/null

apps_remaining=$(flyctl apps list --json \
  | jq --arg c "${controller_app}" --arg d "${data_app}" --arg r "${runner_app}" \
    '[.[] | select(.Name == $c or .Name == $d or .Name == $r)] | length')
postgres_remaining=$(flyctl mpg list --org "$(jq -r '.organization' "${state_file}")" --json \
  | jq --arg id "${mpg_id}" '[.[] | select(.id == $id)] | length')
[[ "${apps_remaining}" == 0 ]] || { printf 'proof apps remain after cleanup\n' >&2; exit 1; }
[[ "${postgres_remaining}" == 0 ]] || { printf 'proof Postgres remains after cleanup\n' >&2; exit 1; }
rm -f "${material_file}"
jq -n --arg run_id "${run_id}" --arg finished_at "${finished_at}" \
  --arg cleaned_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{terminal:true,run_id:$run_id,proof_finished_at:$finished_at,cleaned_at:$cleaned_at,
    apps_deleted:true,postgres_deleted:true,evidence_retrieved:true}' \
  | tee "${state_dir}/cleanup.json"
