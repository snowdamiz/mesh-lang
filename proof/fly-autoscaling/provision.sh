#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
readonly script_dir
repository_root=$(cd "${script_dir}/../.." && pwd)
readonly repository_root

duration_seconds=3600
region=iad
organization=personal
provision_complete=0
created_apps=()
mpg_id=
state_dir=

timestamp() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() { printf '[%s] %s\n' "$(timestamp)" "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
  printf 'Usage: %s [--duration-seconds 3600] [--region iad] [--org personal]\n' "${0##*/}"
}

rollback() {
  local status=$?
  if (( status == 0 || provision_complete == 1 )); then return; fi
  log 'provisioning failed; removing only resources created for this proof run'
  if [[ -n "${mpg_id}" ]]; then
    flyctl mpg destroy "${mpg_id}" --yes >/dev/null 2>&1 || true
  fi
  if (( ${#created_apps[@]} > 0 )); then
    flyctl apps destroy "${created_apps[@]}" --yes >/dev/null 2>&1 || true
  fi
  if [[ -n "${state_dir}" ]]; then
    printf '%s\n' "${status}" > "${state_dir}/provision-exit-code" 2>/dev/null || true
  fi
  if [[ -n "${material_file:-}" ]]; then
    rm -f "${material_file}"
  fi
}
trap rollback EXIT

while (( $# > 0 )); do
  case "$1" in
    --duration-seconds) duration_seconds=${2:-}; shift 2 ;;
    --region) region=${2:-}; shift 2 ;;
    --org) organization=${2:-}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

if ! [[ "${duration_seconds}" =~ ^[0-9]+$ ]] \
  || (( duration_seconds < 60 || duration_seconds > 3600 )); then
  die '--duration-seconds must be between 60 and 3600'
fi
[[ "${region}" =~ ^[a-z0-9]+$ ]] || die 'invalid Fly region'
[[ "${organization}" =~ ^[a-z0-9-]+$ ]] || die 'invalid Fly organization'

for command in cargo flyctl jq openssl; do
  command -v "${command}" >/dev/null || die "required command not found: ${command}"
done
flyctl auth whoami >/dev/null

run_id=$(date -u +%y%m%d%H%M%S)
readonly run_id
readonly suffix=${run_id}
readonly controller_app="mesh-fly-c-${suffix}"
readonly data_app="mesh-fly-d-${suffix}"
readonly runner_app="mesh-fly-r-${suffix}"
readonly mpg_name="mesh-fly-pg-${suffix}"
readonly cluster_id="mesh-fly-${suffix}"
readonly worker_tag="deployment-${run_id}"
readonly worker_image="registry.fly.io/${data_app}:${worker_tag}"
readonly runner_image="registry.fly.io/${runner_app}:${worker_tag}"
state_dir="${repository_root}/target/proof/fly-autoscaling/${run_id}"
readonly state_dir
readonly material_file="${state_dir}/material.json"
mkdir -p "${state_dir}"
chmod 700 "${state_dir}"

log "building the local proof materializer"
(cd "${repository_root}" && cargo build --locked -p meshc --bin meshc)
readonly meshc="${repository_root}/target/debug/meshc"
"${meshc}" proof fly-autoscaling-materialize \
  --controller-app "${controller_app}" \
  --data-app "${data_app}" \
  --cluster-id "${cluster_id}" \
  --output "${material_file}"

voters=$(jq -r '.voters' "${material_file}")
readonly voters
controller_target=$(jq -r '.controller_target' "${material_file}")
readonly controller_target
controller_seed=$(jq -r '.controller_seed' "${material_file}")
readonly controller_seed

for app in "${controller_app}" "${data_app}" "${runner_app}"; do
  log "creating isolated Fly app ${app}"
  flyctl apps create "${app}" --org "${organization}" >/dev/null
  created_apps+=("${app}")
done

token_expiry_seconds=$(( duration_seconds + 3600 ))
readonly token_expiry_seconds
data_app_token=$(flyctl tokens create deploy --app "${data_app}" \
  --name "mesh-proof-${run_id}-data" --expiry "${token_expiry_seconds}s")
readonly data_app_token
controller_app_token=$(flyctl tokens create deploy --app "${controller_app}" \
  --name "mesh-proof-${run_id}-controller" --expiry "${token_expiry_seconds}s")
readonly controller_app_token

log "creating isolated Fly Managed Postgres ${mpg_name}"
flyctl mpg create --name "${mpg_name}" --org "${organization}" --region "${region}" \
  --plan basic --pg-major-version 16 --volume-size 10 --v2 >/dev/null
for _ in $(seq 1 90); do
  mpg_id=$(flyctl mpg list --org "${organization}" --json \
    | jq -r --arg name "${mpg_name}" '.[] | select(.name == $name) | .id' | head -n 1)
  if [[ -n "${mpg_id}" ]]; then
    mpg_status=$(flyctl mpg status "${mpg_id}" --json | jq -r '.data.status // .status // empty')
    [[ "${mpg_status}" == ready ]] && break
  fi
  sleep 5
done
[[ -n "${mpg_id}" ]] || die 'managed Postgres ID was not discoverable'
[[ "${mpg_status:-}" == ready ]] || die "managed Postgres did not become ready: ${mpg_status:-unknown}"

mpg_database=$(flyctl mpg status "${mpg_id}" --json | jq -r '.credentials.dbname // empty')
mpg_user=$(flyctl mpg status "${mpg_id}" --json | jq -r '.credentials.user // empty')
[[ -n "${mpg_database}" && -n "${mpg_user}" ]] || die 'managed Postgres default database identity unavailable'
for app in "${controller_app}" "${data_app}" "${runner_app}"; do
  flyctl mpg attach "${mpg_id}" --app "${app}" --database "${mpg_database}" \
    --username "${mpg_user}" --variable-name DATABASE_URL >/dev/null
done

import_shared_secrets() {
  local app=$1
  {
    printf 'MESH_CLUSTER_ID=%s\n' "${cluster_id}"
    printf 'MESH_CLUSTER_COOKIE=%s\n' "$(jq -r '.cookie' "${material_file}")"
    printf 'MESH_OPERATOR_KEY=%s\n' "$(jq -r '.operator_key' "${material_file}")"
    printf 'MESH_TLS_CA_DER_B64=%s\n' "$(jq -r '.tls_ca_der_b64' "${material_file}")"
    printf 'MESH_TLS_CERT_DER_B64=%s\n' "$(jq -r '.tls_cert_der_b64' "${material_file}")"
    printf 'MESH_TLS_KEY_DER_B64=%s\n' "$(jq -r '.tls_key_der_b64' "${material_file}")"
    printf 'MESH_NODE_IDENTITY_VERIFY_KEYS_B64=%s\n' "$(jq -r '.identity_verify_key_b64' "${material_file}")"
  } | flyctl secrets import --stage --app "${app}" >/dev/null
}
import_shared_secrets "${controller_app}"
import_shared_secrets "${data_app}"
import_shared_secrets "${runner_app}"

{
  printf 'FLY_API_TOKEN=%s\n' "${data_app_token}"
  printf 'MESH_CAPACITY_IDENTITY_SIGNING_KEY_DER_B64=%s\n' \
    "$(jq -r '.identity_signing_key_der_b64' "${material_file}")"
} | flyctl secrets import --stage --app "${controller_app}" >/dev/null
{
  printf 'FLY_DATA_API_TOKEN=%s\n' "${data_app_token}"
  printf 'FLY_CONTROLLER_API_TOKEN=%s\n' "${controller_app_token}"
  printf 'MESH_ROLES=operator\n'
  printf 'MESH_STABLE_NODE_ID=%s/operator/fly-proof\n' "${cluster_id}"
  printf 'MESH_NODE_IDENTITY_ENVELOPE_B64=%s\n' "$(jq -r '.identities.operator' "${material_file}")"
} | flyctl secrets import --stage --app "${runner_app}" >/dev/null

build_args=(
  --build-arg "DATA_APP=${data_app}"
  --build-arg "CONTROLLER_APP=${controller_app}"
  --build-arg "WORKER_IMAGE=${worker_image}"
  --build-arg "CLUSTER_ID=${cluster_id}"
  --build-arg "VOTERS=${voters}"
  --build-arg "REGION=${region}"
  --build-arg "RUN_ID=${run_id}"
)
log "building and pushing immutable application image ${worker_image}"
(cd "${repository_root}" && flyctl deploy --app "${data_app}" \
  --config fly.autoscaling-build.toml \
  --dockerfile proof/fly-autoscaling/Dockerfile --build-target app \
  "${build_args[@]}" --image-label "${run_id}" --build-only --push --remote-only --yes)
log "building and pushing Fly-resident load/evidence runner ${runner_image}"
(cd "${repository_root}" && flyctl deploy --app "${runner_app}" \
  --config fly.autoscaling-build.toml \
  --dockerfile proof/fly-autoscaling/Dockerfile --build-target runner \
  "${build_args[@]}" --image-label "${run_id}" --build-only --push --remote-only --yes)

start_mesh_node() {
  local app=$1 label=$2 role=$3 identity_key=$4 failure_domain=$5
  local stable_id="${cluster_id}/${role}/${label}"
  local node_name="${label}@${label}.mesh_node.kv._metadata.${app}.internal:4370"
  flyctl machine run "${worker_image}" --app "${app}" --name "${label}" --region "${region}" \
    --vm-cpu-kind shared --vm-cpus 1 --vm-memory 512 --restart always --detach \
    --metadata "mesh.proof_run=${run_id}" --metadata "mesh.role=${role}" \
    --metadata "mesh_node=${label}" --metadata "mesh.cluster=${cluster_id}" \
    --metadata "mesh.fixed=true" \
    --env PORT=8080 --env MESH_CLUSTER_MODE=autonomous --env MESH_CLUSTER_PORT=4370 \
    --env "MESH_DISCOVERY_SEED=${controller_seed}" --env "MESH_CONTROLLER_VOTERS=${voters}" \
    --env MESH_ADAPTIVE_ROUTING=true --env MESH_CONTINUITY_DB=/tmp/mesh-continuity.db \
    --env MESH_SCHEDULER_MIN_WORKERS=2 --env MESH_SCHEDULER_MAX_WORKERS=8 \
    --env "MESH_ROLES=${role}" --env "MESH_STABLE_NODE_ID=${stable_id}" \
    --env "MESH_NODE_NAME=${node_name}" --env "MESH_FAILURE_DOMAIN=${failure_domain}" \
    --env "MESH_NODE_IDENTITY_ENVELOPE_B64=$(jq -r ".identities.${identity_key}" "${material_file}")" \
    >/dev/null
}

for ordinal in 1 2 3; do
  start_mesh_node "${controller_app}" "controller-${ordinal}" controller \
    "controller${ordinal}" "controller-${ordinal}"
done
for ordinal in 1 2; do
  start_mesh_node "${data_app}" "gateway-${ordinal}" gateway \
    "gateway${ordinal}" "gateway-${ordinal}"
  start_mesh_node "${data_app}" "worker-${ordinal}" worker \
    "worker${ordinal}" "worker-${ordinal}"
done

runner_volume=$(flyctl volumes create mesh_proof_evidence --app "${runner_app}" --region "${region}" \
  --size 1 --json --yes | jq -r '.. | .id? // empty' | head -n 1)
[[ -n "${runner_volume}" ]] || die 'runner evidence volume creation failed'
readonly runner_volume
readonly gateway_one="http://gateway-1.mesh_node.kv._metadata.${data_app}.internal:8080"
readonly gateway_two="http://gateway-2.mesh_node.kv._metadata.${data_app}.internal:8080"
flyctl machine run "${runner_image}" --app "${runner_app}" --name proof-runner --region "${region}" \
  --vm-cpu-kind shared --vm-cpus 1 --vm-memory 1024 --restart no --detach \
  --volume "${runner_volume}:/data" --metadata "mesh.proof_run=${run_id}" \
  --metadata mesh.role=runner --env "MESH_PROOF_RUN_ID=${run_id}" \
  --env "MESH_PROOF_DURATION_SECONDS=${duration_seconds}" \
  --env "MESH_CONTROLLER_TARGET=${controller_target}" \
  --env "MESH_CONTROLLER_APP=${controller_app}" --env "MESH_DATA_APP=${data_app}" \
  --env "MESH_GATEWAY_ONE=${gateway_one}" --env "MESH_GATEWAY_TWO=${gateway_two}" \
  >/dev/null
runner_machine=$(flyctl machine list --app "${runner_app}" --json \
  | jq -r '.[] | select(.name == "proof-runner") | .id' | head -n 1)
[[ -n "${runner_machine}" ]] || die 'runner Machine was not discoverable'

jq -n \
  --arg run_id "${run_id}" --arg cluster_id "${cluster_id}" --arg region "${region}" \
  --arg organization "${organization}" --arg controller_app "${controller_app}" \
  --arg data_app "${data_app}" --arg runner_app "${runner_app}" \
  --arg mpg_id "${mpg_id}" --arg mpg_name "${mpg_name}" \
  --arg runner_machine "${runner_machine}" --arg runner_volume "${runner_volume}" \
  --arg material_file "${material_file}" --arg started_at "$(timestamp)" \
  --argjson duration_seconds "${duration_seconds}" \
  '{schema_version:1,run_id:$run_id,cluster_id:$cluster_id,region:$region,
    organization:$organization,controller_app:$controller_app,data_app:$data_app,
    runner_app:$runner_app,mpg_id:$mpg_id,mpg_name:$mpg_name,
    runner_machine:$runner_machine,runner_volume:$runner_volume,
    material_file:$material_file,provisioned_at:$started_at,duration_seconds:$duration_seconds}' \
  > "${state_dir}/state.json"
chmod 600 "${state_dir}/state.json"
provision_complete=1
log "proper Fly proof started: ${run_id}"
printf 'state_dir: %s\n' "${state_dir}"
printf 'controller_app: %s\ndata_app: %s\nrunner_app: %s\npostgres: %s (%s)\n' \
  "${controller_app}" "${data_app}" "${runner_app}" "${mpg_name}" "${mpg_id}"
