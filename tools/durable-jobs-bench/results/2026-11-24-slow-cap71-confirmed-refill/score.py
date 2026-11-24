"""Replay sealed slow cap-71 arms and decide whether the pilot earns expansion."""
from collections import Counter
import json
from pathlib import Path
import runpy
import sys
import tarfile

HERE = Path(__file__).resolve().parent
CAP71 = HERE.parent / "2026-11-24-ordinary-slow-equal-topology-cap71"
sys.path.insert(0, str(CAP71))
import qualification as stock

run = stock.run
require = stock.require
window = runpy.run_path(str(HERE.parent / "2026-11-23-iteration-comparison/slow-equal-refill.py"))["window"]
ROLE_CAPS = {"parity_subject": 8, "compiled_caller": 8, "parity_producer": 1}
METRICS = ("middle_ms", "submit_ms", "drain_ms")
GROUP = ("group-a", "group-b")
FUSED = ("fused-a", "fused-b")


def sql_totals(rows, fragment, role):
    matched = [r for r in rows if r["role"] == role and fragment in " ".join(r["query"].lower().split())]
    return {field: sum(r[field] for r in matched) for field in ("calls", "rows", "total_exec_time")}


def workload(jobs, commit, score):
    ranks, areas = window(jobs, commit)
    return dict(score, middle_ms=areas[0]/1000, outside_slot_ms=areas[-1]/1000,
                application_slot_ms=[value/1000 for value in areas[1:-1]],
                first_write_ranks_ms=dict(zip((1, 64, 128, 192, 256, 320, 384, 448, 512),
                                              (value/1000 for value in ranks), strict=True)))


def score_mesh(folder, label, phase, lane, binding):
    spec = lane["inventory"]["engines"]["mesh"]
    image = lane["image"]
    manifest = binding["manifests"][image]
    digest = manifest["files"]["mesh"]
    producer = binding["manifests"]["public"]["files"]["mesh"]
    path = folder / "raw.tar.gz"
    request = run.read_json(folder / "request.json")
    with tarfile.open(path) as archive:
        def files(name):
            return run.file_json(archive, "adapter/" + name)

        def text(name):
            with archive.extractfile(name) as stream:
                return stream.read().decode()

        invoke = run.file_json(archive, "invocation.json")
        expected = dict(phase=phase, profile="slow", engine="mesh", attempt=request["attempt"])
        require(all(invoke[k] == request[k] == v for k, v in expected.items())
                and run.file_json(archive, "status.json")["exit_code"] == 0,
                "archive invocation or arm exit differs")
        host = run.file_json(archive, "host.json")
        design = binding["design"]
        require(host["affinity"] == design["cpus"] and host["verified_files"] == spec["verified_files"]
                and [host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == design["host"],
                "CPU pin, executable files, or host differs")
        ledger = files("ledger.json")
        require(ledger["phase"] == phase and ledger["local_smoke"] is False
                and ledger["execution_contract"] == "ordinary-trusted-shared-process-v2"
                and ledger["queue_policy"] == "success_cohort" and files("build.json") == manifest,
                "unexpected frozen product path")
        artifact = {k: spec[k] for k in ("version", "artifact_sha256")}
        row = stock.reconciler()(ledger, "mesh", "slow", artifact)
        jobs = ledger["jobs"]
        txn, = ledger["transactions"]
        require(row["completed"] == 512 and all(j["hold_ms"] == 100 for j in jobs),
                "canonical durable workload did not finish")
        app = {stock.xid(j[k], k) for j in jobs for k in ("attempt_xid", "effect_xid")}
        cohorts = Counter(stock.xid(j["settled_xid"], "settled_xid") for j in jobs)
        require(len(app) == 1024 and app.isdisjoint(cohorts) and stock.xid(txn["xid"], "producer") not in app | cohorts.keys()
                and max(cohorts.values()) <= 8, "independent application commits or terminal cohorts differ")
        require(files("capacity-before.json") == files("capacity-after.json"), "persisted capacity drifted")
        caps = files("capacity-before.json")
        queue, = caps["sql_queues"]
        require(caps["role_caps"] == ROLE_CAPS
                and (queue["name"], queue["concurrency"], queue["state"]) == ("default", 64, "active"),
                "persisted global queue or role caps differ")
        storage = files("storage-topology.json")
        require({k: storage[k] for k in ("args_storage", "storage", "work_mem", "max_connections", "topology")} ==
                dict(args_storage="x", storage="default", work_mem="4MB", max_connections=71,
                     topology="slow-equal-cap71")
                and files("canonical-storage.json") == dict(rows=512, compressed=0, min_bytes=16384, max_bytes=16384),
                "canonical default-storage payload differs")
        postgres = files("postgres-affinity.json")
        require(postgres["cpus"] == design["cpus"], "wrong PostgreSQL CPU placement")
        roster = stock.ownership(files, ledger, "mesh", postgres)
        pids = {r["pid"] for r in roster}
        sessions = files("postgres-tls-sessions.json")
        require(len(sessions) == 17 and Counter(r["role"] for r in sessions if r["tls"]) == ROLE_CAPS,
                "TLS session allocation differs")
        sockets = files("application-sockets.json")
        require(set(map(int, sockets)) == pids and all(len(backends) == 4 for backends in sockets.values())
                and {backend for backends in sockets.values() for backend in backends} ==
                {r["backend"] for r in sessions if r["role"] == "compiled_caller"},
                "each runner must own four distinct warmed app sockets")
        broker, *runners = files("process-identity.json")
        require({r["pid"] for r in runners} == pids and broker["pid"] not in pids
                and broker["executable_sha256"] == producer and broker["argv"][-1] == "session"
                and all(r["executable_sha256"] == digest and r["argv"][-1] == "run" for r in runners),
                "wrong live runner or canonical producer executable")
        require(all(r["cpus"] == design["cpus"] and
                    r["launch_start_ticks"] == r["start_ticks_before"] == r["start_ticks_after"]
                    for r in [broker, *runners]), "live process identity changed")
        census = stock.physical(text("adapter/physical-postgres.log"), "mesh", row["first_submit_us"],
                                row["last_settlement_us"])
        require(census["whole_cluster_peak"] >= ledger["peak_connections"]
                and census["by_user"] == ROLE_CAPS and stock.hold_peak(jobs) <= 64,
                "physical or global slot bound exceeded")
        lifecycle = files("sql-lifecycle.json")
        require(lifecycle["post_arm"] == dict(cap=71, dealloc=0, sessions={"parity_owner": 1}),
                "lingering clients, changed cap, or statement eviction")
        statements = lifecycle["statements"]
        bulk = sql_totals(statements, "select ordinal, receipt from mesh_jobs.enqueue_batch_v1(", "parity_producer")
        scalar = sql_totals(statements, "select mesh_jobs.enqueue(", "parity_producer")
        require((bulk["calls"], bulk["rows"], scalar["calls"], scalar["rows"]) == (8, 504, 8, 8),
                "canonical producer batching changed")
        for suffix in ("attempts", "effects"):
            writes = sql_totals(statements, f"insert into public.mesh_jobs_parity_{suffix}", "compiled_caller")
            require(writes["calls"] == writes["rows"] == 512, "application SQL ledger changed")
        probes = [r for r in statements if r["role"] == "compiled_caller" and r["query"] == "SELECT $1"]
        require(sum(r["calls"] for r in probes) == 1024, "D2-off must retain every checkout probe")
        claim = sql_totals(statements, "from mesh_jobs.claim_batch_set_v1(", "parity_subject")
        grouped = sql_totals(statements, "from mesh_jobs.complete_batch_v1(", "parity_subject")
        fused = sql_totals(statements, "from mesh_jobs.complete_and_refill_v1(", "parity_subject")
        fallback = sql_totals(statements, "from mesh_jobs.claim_batch_v1(", "parity_subject")
        trace = files("refill-ledger.json")
        require(trace["attempted_at_ms"].keys() == {j["id"] for j in jobs}, "claim timestamp inventory differs")
        reports = trace["runner_status"]
        require(len(reports) == 2 and len({r["runner_id"] for r in reports}) == 2
                and sum(r["claimed"] for r in reports) == sum(r["finished"] for r in reports) == 512
                and all(r["last_error"] is None and r["drained"] and r["stale_acks"] == 0
                        and r["queues"]["default"]["limit"] == 32 for r in reports),
                "both runners must finish and drain within their local slot caps")
        refill = [r.get("completion_refill") for r in reports]
        if label in FUSED:
            require(all(r and r["grants"] > 0 and r["completions"] > 0 for r in refill)
                    and fused["calls"] == sum(r["commits"] for r in refill) > 0,
                    "both fused runners must execute confirmed replacements")
        else:
            require(not fused["calls"] and refill == [None, None] and grouped["calls"] > 0,
                    "group control did not exercise grouped unfused settlement")
        require(claim["calls"] > 0, "no checked initial set claims")
        row.update(physical=census, roster=roster, terminal_xids=len(cohorts),
                   submit_ms=(row["last_settlement_us"]-row["first_submit_us"])/1000,
                   drain_ms=(row["last_settlement_us"]-row["first_commit_us"])/1000,
                   refill=refill, sql=dict(claim=claim, grouped=grouped, fused=fused, cursor=fallback,
                                           producer_bulk=bulk, producer_scalar=scalar),
                   raw_sha256=run.digest(path))
        samples = [json.loads(line) for line in text("host-samples.jsonl").splitlines()]
        row.update(run.sample_window(host, samples, row))
        return workload(jobs, txn["committed_us"], row)


def score_arm(folder, label, phase, binding):
    lane = binding["lanes"][label]
    if lane["engine"] == "mesh":
        return score_mesh(folder, label, phase, lane, binding)
    row = stock.score(folder, lane["engine"], phase, lane["inventory"], binding["design"])
    with tarfile.open(folder / "raw.tar.gz") as archive:
        ledger = run.file_json(archive, "adapter/ledger.json")
    return workload(ledger["jobs"], row["first_commit_us"], row)


def gate(measured):
    controls = (*GROUP, "public-group")
    trials = {}
    for metric in METRICS:
        best = min(measured[lane][metric] for lane in controls)
        worst = max(measured[lane][metric] for lane in FUSED)
        spread = max(abs(measured[a][metric] - measured[b][metric]) for a, b in (GROUP, FUSED))
        saving = best - worst
        threshold = max(best * 0.05, spread)
        trials[metric] = dict(saving_ms=saving, min_required_ms=threshold, pass_gate=saving > threshold)
    outside = min(measured[lane]["outside_slot_ms"] for lane in controls) - max(
        measured[lane]["outside_slot_ms"] for lane in FUSED)
    grants = min(sum(r["grants"] for r in measured[lane]["refill"]) for lane in FUSED)
    passes = all(row["pass_gate"] for row in trials.values()) and outside > 0 and grants >= 90
    return dict(advance=passes, metrics=trials, outside_saving_ms=outside, minimum_fused_grants=grants)
