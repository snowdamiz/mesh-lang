#!/usr/bin/env bash
set -uo pipefail

readonly evidence_dir=/data/evidence
readonly results_dir="${evidence_dir}/requests"
readonly duration_seconds="${MESH_PROOF_DURATION_SECONDS:-3600}"
readonly checkpoint_seconds="${MESH_PROOF_CHECKPOINT_SECONDS:-60}"
readonly phase_seconds="${MESH_PROOF_PHASE_SECONDS:-450}"
readonly controller_target="${MESH_CONTROLLER_TARGET:-}"
readonly gateway_one="${MESH_GATEWAY_ONE:-}"
readonly gateway_two="${MESH_GATEWAY_TWO:-}"
readonly data_app="${MESH_DATA_APP:-}"
readonly controller_app="${MESH_CONTROLLER_APP:-}"
readonly run_id="${MESH_PROOF_RUN_ID:-}"
readonly machines_api=https://api.machines.dev/v1/apps

load_pids=()

timestamp() { date -u +%Y-%m-%dT%H:%M:%SZ; }
epoch_millis() { date +%s%3N; }
log() { printf '[%s] %s\n' "$(timestamp)" "$*" | tee -a "${evidence_dir}/runner.log"; }

required_environment() {
  local name
  for name in DATABASE_URL FLY_DATA_API_TOKEN FLY_CONTROLLER_API_TOKEN \
    MESH_CLUSTER_COOKIE MESH_TLS_CA_DER_B64 \
    MESH_TLS_CERT_DER_B64 MESH_TLS_KEY_DER_B64 MESH_NODE_IDENTITY_VERIFY_KEYS_B64 \
    MESH_NODE_IDENTITY_ENVELOPE_B64 MESH_STABLE_NODE_ID MESH_CLUSTER_ID; do
    if [[ -z "${!name:-}" ]]; then
      printf 'missing required environment: %s\n' "${name}" >&2
      return 1
    fi
  done
  if [[ -z "${controller_target}" || -z "${gateway_one}" || -z "${gateway_two}" \
    || -z "${data_app}" || -z "${controller_app}" || -z "${run_id}" ]]; then
    printf 'missing proof topology environment\n' >&2
    return 1
  fi
  if ! [[ "${duration_seconds}" =~ ^[0-9]+$ ]] \
    || (( duration_seconds < 60 || duration_seconds > 3600 )); then
    printf 'duration must be between 60 and 3600 seconds\n' >&2
    return 1
  fi
}

api_get_machines() {
  local app=$1
  local token
  if [[ "${app}" == "${data_app}" ]]; then
    token=${FLY_DATA_API_TOKEN}
  elif [[ "${app}" == "${controller_app}" ]]; then
    token=${FLY_CONTROLLER_API_TOKEN}
  else
    printf 'refusing Machines API access outside proof apps: %s\n' "${app}" >&2
    return 1
  fi
  curl --fail --silent --show-error --max-time 15 \
    -H "Authorization: Bearer ${token}" \
    "${machines_api}/${app}/machines"
}

checkpoint() {
  local now=$1
  local capacity='null' data_machines='[]' controller_machines='[]' pg_count='null'
  local gateway_one_status=0 gateway_two_status=0
  capacity=$(meshc cluster capacity "${controller_target}" --json --timeout-ms 5000 2>/dev/null) || true
  data_machines=$(api_get_machines "${data_app}" 2>/dev/null) || true
  controller_machines=$(api_get_machines "${controller_app}" 2>/dev/null) || true
  pg_count=$(psql "${DATABASE_URL}" -Atqc 'SELECT COUNT(*) FROM todos' 2>/dev/null) || true
  gateway_one_status=$(curl --silent --output /dev/null --max-time 5 --write-out '%{http_code}' "${gateway_one}/health" 2>/dev/null) || gateway_one_status=0
  gateway_two_status=$(curl --silent --output /dev/null --max-time 5 --write-out '%{http_code}' "${gateway_two}/health" 2>/dev/null) || gateway_two_status=0
  jq -cn \
    --arg observed_at "$(timestamp)" \
    --argjson elapsed "$(( now - proof_started_epoch ))" \
    --argjson capacity "${capacity:-null}" \
    --argjson data_machines "${data_machines:-[]}" \
    --argjson controller_machines "${controller_machines:-[]}" \
    --argjson pg_count "${pg_count:-null}" \
    --argjson gateway_one_status "${gateway_one_status:-0}" \
    --argjson gateway_two_status "${gateway_two_status:-0}" \
    '{observed_at:$observed_at,elapsed_seconds:$elapsed,capacity:$capacity,
      data_machines:$data_machines,controller_machines:$controller_machines,
      postgres_todos:$pg_count,gateway_one_status:$gateway_one_status,
      gateway_two_status:$gateway_two_status}' \
    >> "${evidence_dir}/checkpoints.jsonl"
}

load_worker() {
  local ordinal=$1 end_epoch=$2
  local output="${results_dir}/worker-${ordinal}.tsv"
  local request=0 gateway header_file response status total execution ingress started latency
  while (( $(date +%s) < end_epoch )); do
    if (( (request + ordinal) % 2 == 0 )); then gateway=${gateway_one}; else gateway=${gateway_two}; fi
    header_file=$(mktemp /tmp/mesh-fly-headers.XXXXXX)
    started=$(epoch_millis)
    response=$(curl --silent --show-error --max-time 15 \
      --dump-header "${header_file}" --output /dev/null \
      --write-out '%{http_code}\t%{time_total}' \
      "${gateway}/proof/pressure" 2>/dev/null) || response=$'000\t15.000'
    status=${response%%$'\t'*}
    total=${response#*$'\t'}
    latency=$(( $(epoch_millis) - started ))
    execution=$(awk -F ': ' 'tolower($1)=="x-mesh-execution-node" {gsub("\\r", "", $2); print $2; exit}' "${header_file}")
    ingress=$(awk -F ': ' 'tolower($1)=="x-mesh-ingress-node" {gsub("\\r", "", $2); print $2; exit}' "${header_file}")
    rm -f "${header_file}"
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$(timestamp)" "${gateway}" "${status}" "${latency}" "${total}" \
      "${ingress:--}" "${execution:--}" >> "${output}"
    request=$(( request + 1 ))
  done
}

start_load() {
  local end_epoch=$1 ordinal
  load_pids=()
  for ordinal in $(seq 1 16); do
    load_worker "${ordinal}" "${end_epoch}" &
    load_pids+=("$!")
  done
}

stop_load() {
  local pid
  for pid in "${load_pids[@]:-}"; do
    kill "${pid}" 2>/dev/null || true
  done
  for pid in "${load_pids[@]:-}"; do
    wait "${pid}" 2>/dev/null || true
  done
  load_pids=()
}

record_postgres_mutation() {
  local phase=$1 gateway response status
  if (( phase % 2 == 0 )); then gateway=${gateway_one}; else gateway=${gateway_two}; fi
  response=$(curl --silent --show-error --max-time 15 --output /dev/null \
    --write-out '%{http_code}' -H 'Content-Type: application/json' \
    --data "{\"title\":\"fly-proof-${run_id}-phase-${phase}\"}" \
    "${gateway}/todos" 2>/dev/null) || response=000
  status=${response:-000}
  printf '%s\t%s\t%s\t%s\n' "$(timestamp)" "${phase}" "${gateway}" "${status}" \
    >> "${evidence_dir}/postgres-mutations.tsv"
}

inject_worker_loss_once() {
  local machine_id
  machine_id=$(api_get_machines "${data_app}" \
    | jq -r '[.[] | select(.config.metadata["mesh.managed"] == "true" and .state == "started")][0].id // empty') || true
  if [[ -z "${machine_id}" ]]; then
    log 'fault injection deferred: no started dynamically managed worker'
    return 1
  fi
  if curl --fail --silent --show-error --max-time 20 -X DELETE \
    -H "Authorization: Bearer ${FLY_DATA_API_TOKEN}" \
    "${machines_api}/${data_app}/machines/${machine_id}?force=true" >/dev/null; then
    jq -cn --arg at "$(timestamp)" --arg machine_id "${machine_id}" \
      --argjson elapsed_seconds "$(( $(date +%s) - proof_started_epoch ))" \
      '{at:$at,elapsed_seconds:$elapsed_seconds,type:"managed_worker_forced_loss",machine_id:$machine_id}' \
      > "${evidence_dir}/fault-injection.json"
    log "injected loss of dynamically managed worker ${machine_id}"
    return 0
  fi
  return 1
}

percentile() {
  local file=$1 percentile=$2 count index
  count=$(wc -l < "${file}" | tr -d ' ')
  if (( count == 0 )); then printf '0'; return; fi
  index=$(( (count * percentile + 99) / 100 ))
  (( index < 1 )) && index=1
  sed -n "${index}p" "${file}"
}

summarize() {
  local combined="${evidence_dir}/requests.tsv" latency_file="${evidence_dir}/latencies-ms.txt"
  local requests successes failures gateway_one_count gateway_two_count execution_nodes
  local capacity_max capacity_min capacity_final managed_max mutation_successes pg_rows
  local controllers_max gateways_max baseline_workers_max fault_recovered=false fault_elapsed
  find "${results_dir}" -type f -name '*.tsv' -print0 | sort -z | xargs -0 cat > "${combined}"
  awk -F '\t' '{print $4}' "${combined}" | sort -n > "${latency_file}"
  requests=$(wc -l < "${combined}" | tr -d ' ')
  successes=$(awk -F '\t' '$3 == 200 {count++} END {print count+0}' "${combined}")
  failures=$(( requests - successes ))
  gateway_one_count=$(awk -F '\t' -v gateway="${gateway_one}" '$2 == gateway && $3 == 200 {count++} END {print count+0}' "${combined}")
  gateway_two_count=$(awk -F '\t' -v gateway="${gateway_two}" '$2 == gateway && $3 == 200 {count++} END {print count+0}' "${combined}")
  execution_nodes=$(awk -F '\t' '$3 == 200 && $7 != "-" {print $7}' "${combined}" | sort -u | wc -l | tr -d ' ')
  capacity_max=$(jq -s '[.[].capacity.ready // 0] | max // 0' "${evidence_dir}/checkpoints.jsonl")
  capacity_min=$(jq -s '[.[].capacity.ready // 0] | min // 0' "${evidence_dir}/checkpoints.jsonl")
  capacity_final=$(tail -n 1 "${evidence_dir}/checkpoints.jsonl" | jq '.capacity.ready // 0')
  managed_max=$(jq -s '[.[] | [.data_machines[] | select(.config.metadata["mesh.managed"] == "true" and .state == "started")] | length] | max // 0' "${evidence_dir}/checkpoints.jsonl")
  controllers_max=$(jq -s '[.[] | [.controller_machines[] | select(.config.metadata["mesh.role"] == "controller" and .state == "started")] | length] | max // 0' "${evidence_dir}/checkpoints.jsonl")
  gateways_max=$(jq -s '[.[] | [.data_machines[] | select(.config.metadata["mesh.role"] == "gateway" and .state == "started")] | length] | max // 0' "${evidence_dir}/checkpoints.jsonl")
  baseline_workers_max=$(jq -s '[.[] | [.data_machines[] | select(.config.metadata["mesh.role"] == "worker" and .state == "started")] | length] | max // 0' "${evidence_dir}/checkpoints.jsonl")
  mutation_successes=$(awk -F '\t' '$4 == 201 {count++} END {print count+0}' "${evidence_dir}/postgres-mutations.tsv")
  pg_rows=$(psql "${DATABASE_URL}" -Atqc "SELECT COUNT(*) FROM todos WHERE title LIKE 'fly-proof-${run_id}-%'")
  if [[ -f "${evidence_dir}/fault-injection.json" ]]; then
    fault_elapsed=$(jq '.elapsed_seconds' "${evidence_dir}/fault-injection.json")
    fault_recovered=$(jq -s --argjson fault_elapsed "${fault_elapsed}" \
      'any(.[]; .elapsed_seconds > $fault_elapsed and (.capacity.ready // 0) >= 5)' \
      "${evidence_dir}/checkpoints.jsonl")
  fi
  local success_rate_bps=0
  if (( requests > 0 )); then success_rate_bps=$(( successes * 10000 / requests )); fi
  jq -n \
    --arg run_id "${run_id}" \
    --arg started_at "$(cat "${evidence_dir}/started-at")" \
    --arg finished_at "$(timestamp)" \
    --argjson duration_seconds "${duration_seconds}" \
    --argjson requests "${requests}" \
    --argjson successes "${successes}" \
    --argjson failures "${failures}" \
    --argjson success_rate_bps "${success_rate_bps}" \
    --argjson gateway_one_successes "${gateway_one_count}" \
    --argjson gateway_two_successes "${gateway_two_count}" \
    --argjson execution_nodes "${execution_nodes}" \
    --argjson p50 "$(percentile "${latency_file}" 50)" \
    --argjson p95 "$(percentile "${latency_file}" 95)" \
    --argjson p99 "$(percentile "${latency_file}" 99)" \
    --argjson capacity_min "${capacity_min}" \
    --argjson capacity_max "${capacity_max}" \
    --argjson capacity_final "${capacity_final}" \
    --argjson managed_dynamic_max "${managed_max}" \
    --argjson controllers_max "${controllers_max}" \
    --argjson gateways_max "${gateways_max}" \
    --argjson baseline_workers_max "${baseline_workers_max}" \
    --argjson mutation_successes "${mutation_successes}" \
    --argjson postgres_rows "${pg_rows}" \
    --argjson fault_injected "$(test -f "${evidence_dir}/fault-injection.json" && printf true || printf false)" \
    --argjson fault_recovered "${fault_recovered}" \
    '{schema_version:1,run_id:$run_id,started_at:$started_at,finished_at:$finished_at,
      duration_seconds:$duration_seconds,load:{requests:$requests,successes:$successes,
      failures:$failures,success_rate_bps:$success_rate_bps,gateway_one_successes:$gateway_one_successes,
      gateway_two_successes:$gateway_two_successes,execution_nodes:$execution_nodes,
      latency_millis:{p50:$p50,p95:$p95,p99:$p99}},capacity:{min_ready:$capacity_min,
      max_ready:$capacity_max,final_ready:$capacity_final,max_dynamic_machines:$managed_dynamic_max},
      postgres:{successful_mutations:$mutation_successes,matching_rows:$postgres_rows},
      topology:{maximum_started_controllers:$controllers_max,maximum_started_gateways:$gateways_max,
      maximum_started_baseline_workers:$baseline_workers_max},fault_injected:$fault_injected,
      assertions:{one_hour_exact:($duration_seconds == 3600),three_controllers:($controllers_max == 3),
      two_fixed_gateways:($gateways_max == 2),two_baseline_workers:($baseline_workers_max == 2),
      two_gateways:($gateway_one_successes > 0 and $gateway_two_successes > 0),
      scaled_up:($capacity_max >= 5),scaled_down:($capacity_final <= 2),
      multiple_execution_nodes:($execution_nodes >= 2),success_rate:($success_rate_bps >= 9500),
      postgres_exact:($mutation_successes > 0 and $postgres_rows == $mutation_successes),
      managed_worker_recovered:($fault_injected and $fault_recovered)}}' > "${evidence_dir}/summary.json"
  jq -e '.assertions | to_entries | all(.value == true)' "${evidence_dir}/summary.json" >/dev/null
}

run_proof() {
  required_environment
  mkdir -p "${evidence_dir}" "${results_dir}"
  : > "${evidence_dir}/checkpoints.jsonl"
  : > "${evidence_dir}/postgres-mutations.tsv"
  printf '%s\n' "$(timestamp)" > "${evidence_dir}/started-at"
  psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 -f /usr/local/share/mesh-proof/init.sql \
    > "${evidence_dir}/postgres-init.log" 2>&1
  proof_started_epoch=$(date +%s)
  readonly proof_started_epoch
  local deadline=$(( proof_started_epoch + duration_seconds ))
  local next_checkpoint=${proof_started_epoch} active_phase=-1 fault_attempted=0 phase phase_end now
  log "one-hour Fly autoscaling proof started; deadline epoch ${deadline}"
  while (( (now=$(date +%s)) < deadline )); do
    phase=$(( (now - proof_started_epoch) / phase_seconds ))
    if (( phase != active_phase )); then
      stop_load
      active_phase=${phase}
      phase_end=$(( proof_started_epoch + (phase + 1) * phase_seconds ))
      (( phase_end > deadline )) && phase_end=${deadline}
      record_postgres_mutation "${phase}"
      if (( phase % 2 == 0 )); then
        log "phase ${phase}: high load through both gateways"
        start_load "${phase_end}"
      else
        log "phase ${phase}: idle scale-down observation"
      fi
    fi
    if (( now >= next_checkpoint )); then
      checkpoint "${now}"
      next_checkpoint=$(( now + checkpoint_seconds ))
    fi
    if (( fault_attempted == 0 && now - proof_started_epoch >= duration_seconds / 2 )); then
      if inject_worker_loss_once; then fault_attempted=1; fi
    fi
    sleep 5
  done
  stop_load
  checkpoint "$(date +%s)"
  summarize
}

main() {
  mkdir -p "${evidence_dir}" "${results_dir}"
  local status=0
  run_proof || status=$?
  stop_load
  printf '%s\n' "${status}" > "${evidence_dir}/exit-code"
  printf '%s\n' "$(timestamp)" > "${evidence_dir}/finished-at"
  if (( status == 0 )); then
    touch "${evidence_dir}/release-pass"
    log 'Fly autoscaling proof passed; evidence retained for retrieval'
  else
    touch "${evidence_dir}/release-fail"
    log "Fly autoscaling proof failed with exit ${status}; evidence retained for retrieval"
  fi
  exec sleep infinity
}

main "$@"
