"""Check, stage, or replay the public FILE prepared-claim comparison."""

from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import hashlib
import json
from pathlib import Path
import re
import runpy
from statistics import median
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
ID = HERE.name
FILE_SESSION = HERE.parent / "2026-11-24-public-large-file-session/resume"
PUBLIC_NATIVE = FILE_SESSION / "evidence/native"
RETRY = runpy.run_path(str(FILE_SESSION / "adapter-path-retry/retry.py"))
original, parked, run = RETRY["original"], RETRY["parked"], RETRY["run"]
matrix, intake = original.matrix, original.intake
EVIDENCE = HERE / "evidence"
PACKET = HERE / "request.json"
BUNDLE = HERE / "source.tar.gz"
PROTOCOL = "locked_prepare_pg17_v1"
PEERS = ("river", "oban", "apalis")
CONTROL = ("control-a", "control-b")
TREATMENT = "prepared"
STOCK = "exact-stock"
PILOT = (CONTROL[0], TREATMENT, STOCK, CONTROL[1])
INSTALL = "source/compiler/mesh-rt/src/jobs"
INPUT = "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1"
BROKER = "a5a2d5667c2d7209234d7f1468c40dd4071818010be31364a4e82d04b7ff18a3"
EXPECTED = next(iter(intake.verify.expected_inputs("large").values()))


def sha(path):
    return run.digest(path)


def packet():
    doc = run.read_json(PACKET)
    assert doc["id"] == ID and doc["input_sha256"] == INPUT
    assert sha(BUNDLE) == doc["source_sha256"]
    for path, digest in doc["pins"].items():
        assert sha(ROOT / path) == digest, path
    return doc


def base():
    doc = RETRY["document"]()
    config = RETRY["plan"](doc, RETRY["RECOVERED"])["configuration"]
    release = run.read_json(RETRY["RECOVERED"] / "controller-release.json")
    assert (release["id"], release["exit_status"]) == (original.ID, 0)
    spec = config["lanes"]["file"]["inventory"]["engines"]["mesh"]
    assert f"PRODUCT_SESSION_SHA256={BROKER}" in spec["argv"]
    assert "PRODUCT_SESSION_MODE=file" in spec["argv"]
    return config


def stage_request(mode):
    path = HERE / (mode + ".json")
    doc = run.read_json(path)
    assert (doc["id"], doc["packet_sha256"]) == (f"{ID}-{mode}", sha(PACKET))
    return path, doc


def grant(kind, mode):
    path, doc = stage_request(mode)
    queue = parked.QUEUE
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                        (queue / "STATUS.md").read_text(), re.M)
    tag, key, start, end = events[-1]
    assert (tag, key) == (kind, doc["id"]) and end
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    assert (queue / f"{key}.json").read_bytes() == path.read_bytes()
    return dict(id=key, start=start, end=end, request_sha256=sha(path))


def released(mode):
    _, doc = stage_request(mode)
    release = run.read_json(parked.QUEUE / f"{doc['id']}.released.json")
    assert (release["id"], release["exit_status"]) == (doc["id"], 0)
    return release


def remote(doc):
    return parked.BASE / f"source-public-large-exact-prepare-{doc['stage_id']}"


def mesh_proof(get, text, artifact, runner, active, measured):
    rows = get("ledger")["jobs"]
    assert len(rows) == 64 and {r["job_key"]: (r["payload_sha256"], r["hold_ms"], r["fail_first"])
                               for r in rows} == EXPECTED
    assert all(r["attempt"] == r["attempts"] == r["effects"] == 1 and r["state"] == "completed"
               for r in rows)
    assert len({r[k] for r in rows for k in ("settled_xid", "attempt_xid", "effect_xid")}) == 192
    (txn,) = get("ledger")["transactions"]
    assert get("ledger")["input_sha256"] == INPUT
    assert get("build") == artifact
    storage = get("storage-topology")
    assert (storage["storage"], storage["topology"], storage["args_storage"], storage["work_mem"]) == (
        "default", "stock", "x", "4MB")
    settings = get("postgres-settings")
    assert settings["version"] == "17.11" and settings["max_connections"] == 96
    assert all(settings[k] for k in ("tls", "fsync", "full_page_writes", "synchronous_commit"))
    row = get("row")
    assert (row["runners"], row["slots"], row["completed"]) == (8, 64, 64)
    assert row["peak_executing"] <= 64 and row["peak_connections"] <= 96
    assert get("ordered-writes")["ordered"] == 64
    tls = get("postgres-tls-sessions")
    assert all(r["tls"] for r in tls)
    assert sum(r["role"] == "parity_producer" for r in tls) == 1
    assert sum(r["role"] == "compiled_caller" for r in tls) == 32
    writer = get("producer-writer")
    assert (writer["enabled"], writer["scalar"], writer["bulk"]) == (True, 64, 0)
    assert text("broker-output.log").splitlines()[-1] == b"{}"
    identities = get("process-identity")
    assert len(identities) == 9 and all(r["cpus"] == [0, 1] for r in identities)
    session, = [r for r in identities if r["argv"][-1] == "session"]
    assert session["argv"] == [str(parked.REMOTE_BROKER), "session"]
    assert session["executable_sha256"] == BROKER
    workers = [r for r in identities if r["argv"][-1] == "run"]
    assert len(workers) == 8 and all(r["argv"][0] == str(runner)
        and r["executable_sha256"] == artifact["files"]["mesh"] for r in workers)
    queue = get("queue-policy")
    assert queue["queue_policy"] == "set_based"
    assert queue["terminal_xids"] == 64 and queue["largest_terminal_cohort"] == 1
    assert queue["completion_cohort_queries"] == 0 and queue["admitted_queries"] > 0
    queries = queue["queue_statements"]
    prepares = [q for q in queries if "FROM mesh_jobs.prepare_batch_v1(" in q["query"]]
    activations = [q for q in queries if "FROM mesh_jobs.activate_batch_v1(" in q["query"]]
    assert bool(prepares) == bool(activations) == active
    if active:
        assert sum(q["rows"] for q in prepares) >= 64
        assert sum(q["rows"] - q["calls"] for q in activations) == 64
        assert get("prepare-install") == dict(protocol=PROTOCOL, roles_scoped=True)
        assert all("FROM mesh_jobs.claim_batch_" not in q["query"] for q in queries)
    statements = get("trusted-application-statements")
    for table in ("attempts", "effects"):
        insert, = [q for q in statements if q["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + table)]
        assert insert["calls"] == insert["rows"] == 64
    reports = []
    for i in range(8):
        ready, report = text(f"runner-{i}.log").splitlines()
        assert ready == b"READY"
        status = json.loads(report)
        assert status["drained"] and status["last_error"] is None and status["recovered"] == 0
        assert status["outcomes"]["completed"] == status["finished"] == status["claimed"]
        assert status.get("batch_protocol") == (PROTOCOL if active else None)
        assert "event_logs_dropped" not in status
        assert ("last_batch_confirmed_us" in status) == measured
        if measured:
            assert bool(status["last_batch_confirmed_us"]) == bool(status["claimed"])
        reports.append(status)
    assert sum(r["claimed"] for r in reports) == 64
    last_stamp = max(j["claimed_us"] for j in rows)
    settled = max(j["settled_us"] for j in rows)
    confirmed_us = None
    if measured:
        confirmed = max(r["last_batch_confirmed_us"] for r in reports)
        assert txn["committed_us"] <= last_stamp <= confirmed <= max(j["entered_us"] for j in rows)
        confirmed_us = confirmed - txn["committed_us"]
    temp = get("database-temp")
    assert all(type(temp[k]) is int and temp[k] >= 0 for k in ("bytes", "files"))
    return dict(last_confirmed_us=confirmed_us, last_stamp_us=last_stamp-txn["committed_us"],
                submit_us=settled-txn["submitted_us"], drain_us=settled-txn["committed_us"],
                enqueue_us=txn["committed_us"]-txn["submitted_us"], temp_bytes=temp["bytes"],
                temp_files=temp["files"])


def setup_binding(raw, doc):
    where = remote(doc)
    with tarfile.open(raw) as tar:
        def get(name):
            return tar.extractfile("./" + name).read()

        assert get("exit-status") == b"0\n" and get("SETUP-PASS") == b""
        artifact = json.loads(get("artifact.json"))
        assert (artifact["system"], artifact["architecture"], artifact["compiler_sha256"]) == (
            "Linux", "x86_64", doc["compiler_sha256"])
        assert (artifact["version"], artifact["source_sha256"]) == (doc["revision"], doc["source_sha256"])
        assert artifact["files"]["mesh"] != parked.ELF
        pins = {path: digest for digest, path in
                (line.split() for line in get("installed.sha256").decode().splitlines())}
        assert pins[str(where / "artifact/mesh.json")] == hashlib.sha256(get("artifact.json")).hexdigest()
        for filename in (*doc["adapter_files"], *doc["sql_files"]):
            assert pins[str(where / filename)] == doc["portable_files"][filename]
        assert str(where / "pg17_lock_exact.so") in pins
        assert set(pins) == {str(where / name) for name in (*doc["adapter_files"], *doc["sql_files"],
                                                        "pg17_lock_exact.so", "artifact/mesh.json")}
        smoke = mesh_proof(lambda key: json.loads(get(f"smoke/{key}.json")),
                           lambda name: get(f"smoke/{name}"), artifact, where / "artifact/mesh", True, True)
    return dict(artifact=artifact, installed=pins, smoke=smoke, raw_sha256=sha(raw))


def setup(doc):
    request = grant("SETUP", "setup")
    folder = EVIDENCE / "native-setup"
    folder.mkdir()
    run.write_json(folder / "grant.json", request)
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        grant("SETUP", "setup")
        with BUNDLE.open("rb") as payload, (folder / "raw.tar.gz").open("xb") as raw, \
                (folder / "setup.log").open("xb") as log:
            result = run.ssh(["bash", "-c", (HERE / "native-install.sh").read_text(), "--",
                              doc["source_sha256"], doc["revision"], doc["stage_id"]],
                             stdin=payload, stdout=raw, stderr=log, timeout=1150)
        run.write_json(folder / "status.json", dict(exit_code=result.returncode))
        result.check_returncode()
    proof = setup_binding(folder / "raw.tar.gz", doc)
    run.write_json(folder / "qualified.json", proof)


def inventory(doc, proof, mode):
    current = base()
    where = remote(doc)
    artifact = proof["artifact"]
    lanes = current["lanes"]
    labels = PILOT if mode == "pilot" else (CONTROL[0], TREATMENT, STOCK, *PEERS)
    chosen = {}
    for label in labels:
        if label in PEERS:
            chosen[label] = lanes[label]
            continue
        lane = deepcopy(lanes["file"])
        spec = lane["inventory"]["engines"]["mesh"]
        argv = spec["argv"]
        pos, = [i for i, value in enumerate(argv) if value.endswith("/native-run")]
        old_adapter = Path(argv[pos]).parent
        argv[pos:pos+1] = [f"PRODUCT_CLAIM_MODE={'prepared' if label == TREATMENT else 'stock'}",
                           str(where / "adapter/native-run")]
        pins = {p: digest for p, digest in spec["verified_files"].items() if Path(p).parent != old_adapter}
        for path in doc["adapter_files"]:
            pins[str(where / path)] = proof["installed"][str(where / path)]
        if label != STOCK:
            new = where / "artifact"
            old = parked.STOCK.parent
            manifest = proof["installed"][str(new / "mesh.json")]
            replacement = {
                "PRODUCT_ARTIFACT": str(new),
                "PRODUCT_ELF_SHA256": artifact["files"]["mesh"],
                "PRODUCT_MANIFEST_SHA256": manifest,
            }
            for key, value in replacement.items():
                index, = [i for i, arg in enumerate(argv) if arg.startswith(key + "=")]
                argv[index] = f"{key}={value}"
            pins = {p: digest for p, digest in pins.items() if Path(p).parent != old}
            pins.update({str(new / path): digest for path, digest in artifact["files"].items()})
            pins[str(new / "mesh.json")] = manifest
            spec.update(version=artifact["version"], artifact_sha256=manifest)
        if label == TREATMENT:
            pins.update({str(where / p): proof["installed"][str(where / p)]
                         for p in (*doc["sql_files"], "pg17_lock_exact.so")})
        spec["verified_files"] = pins
        chosen[label] = lane
    return dict(current, lanes=chosen)


def orders(config, mode):
    if mode == "pilot":
        return [dict(group=i, profile="large", phases=["warmup", "measured"], order=list(order))
                for i, order in enumerate((PILOT, PILOT[::-1]), 1)]
    return list(matrix_plan(["large"], list(config["lanes"])))


matrix_plan = matrix.plan


def score_block(path, config, block, doc, proof):
    assert run.read_json(path / "order.json") == block
    scores = {}
    for phase, label, folder in matrix.arms(path, config, block):
        lane = config["lanes"][label]
        engine = lane["engine"]
        result = run.score_arm(folder, block, phase, engine, lane["inventory"])
        assert [result["host_boot_id"], result["host_kernel"], result["pg_mount"]["maj:min"]] == config["host"]
        archive_path = folder / "raw.tar.gz"
        if engine == "mesh":
            exact = label == STOCK
            with tarfile.open(archive_path) as archive:
                get = lambda key: run.file_json(archive, f"adapter/{key}.json")
                text = lambda key: archive.extractfile(f"adapter/{key}").read()
                build = get("build") if exact else proof["artifact"]
                if exact:
                    assert build["files"]["mesh"] == parked.ELF
                runner = parked.STOCK if exact else remote(doc) / "artifact/mesh"
                result.update(mesh_proof(get, text, build, runner, label == TREATMENT, not exact))
        else:
            result.update(submit_us=result["last_settlement_us"] - result["first_submit_us"],
                          drain_us=result["last_settlement_us"] - result["first_commit_us"])
        if phase == "measured":
            result.update(matrix.host_cpu(folder, result, config["clock_ticks"]))
            scores[label] = dict(result, raw_sha256=sha(archive_path))
    return dict(scores=scores)


def gate(scores):
    a, b, candidate, exact = (scores[k] for k in (*CONTROL, TREATMENT, STOCK))
    margins = {}
    for metric, ratio in (("last_confirmed_us", .05), ("drain_us", .05), ("submit_us", .03)):
        best = min(a[metric], b[metric])
        spread = abs(a[metric] - b[metric])
        margin = max(20_000, best * ratio, spread)
        saving = best - candidate[metric]
        bound = None if metric == "last_confirmed_us" else max(20_000, exact[metric] * .05, spread)
        passes = saving > margin and (bound is None or (
            abs(best - exact[metric]) <= bound and exact[metric] - candidate[metric] > margin))
        margins[metric] = dict(saving=saving, spread=spread, margin=margin, passes=passes)
    def pinned_cpu(row):
        return row["pinned_user_cpu_ms_per_job"] + row["pinned_kernel_cpu_ms_per_job"]

    resources = dict(
        cpu=pinned_cpu(candidate) <= max(pinned_cpu(a), pinned_cpu(b)) * 1.20,
        rss=candidate["peak_runner_rss_bytes"] <= max(a["peak_runner_rss_bytes"], b["peak_runner_rss_bytes"]) * 1.25,
        temp=candidate["temp_bytes"] <= max(a["temp_bytes"], b["temp_bytes"]) + 16 * 1024 * 1024,
    )
    return dict(qualifies=all(v["passes"] for v in margins.values()) and all(resources.values()),
                margins=margins, resource_checks=resources)


def bound_setup(doc):
    released("setup")
    folder = EVIDENCE / "native-setup"
    proof = setup_binding(folder / "raw.tar.gz", doc)
    assert proof == run.read_json(folder / "qualified.json")
    return proof


def replay(doc, proof, mode):
    folder = EVIDENCE / f"native-{mode}"
    config = inventory(doc, proof, mode)
    series = run.read_json(folder / "series.json")
    blocks = orders(config, mode)
    assert series["plan"] == blocks and series["config"] == config
    results = []
    for block in blocks:
        selected, = [p for p in matrix.attempt_paths(folder, block) if (p / "reconciled.json").is_file()]
        record = score_block(selected, config, block, doc, proof)
        assert record == run.read_json(selected / "reconciled.json")
        results.append(record["scores"])
    if mode == "pilot":
        gates = [gate(row) for row in results]
        return dict(qualifies=all(row["qualifies"] for row in gates), gates=gates, scores=results)
    summary = {
        k: {metric: dict(median=median(values), min=min(values), max=max(values))
            for metric in ("submit_us", "drain_us")
            for values in [[row[k][metric] for row in results]]}
        for k in config["lanes"]
    }
    return dict(summary=summary, scores=results)


def historical():
    old_release = run.read_json(PUBLIC_NATIVE / "timing-adapter-path-retry-release.json")
    assert (old_release["id"], old_release["exit_status"]) == (RETRY["ID"], 0)
    gap = runpy.run_path(str(HERE.parent / "2026-11-25-public-large-claim-gap/replay.py"))
    assert len(gap["analyze"](gap["SERIES"], 5)) == 15


def timing(doc, mode):
    grant("TIMING", mode)
    proof = bound_setup(doc)
    if mode == "peers":
        released("pilot")
        assert replay(doc, proof, "pilot")["qualifies"]
        historical()
    config = inventory(doc, proof, mode)
    matrix.plan = lambda *_: iter(orders(config, mode))
    matrix.replay = lambda path, cfg, block: score_block(path, cfg, block, doc, proof)
    matrix.ATTEMPT_ERRORS += (AssertionError,)
    dispatch = matrix.dispatch_block

    def dispatch_on_grant(*args):
        grant("TIMING", mode)
        return dispatch(*args)

    matrix.dispatch_block = dispatch_on_grant
    return matrix.dispatch(EVIDENCE / f"native-{mode}", config, [])


def check(doc):
    old = base()
    historical()
    for mode in ("setup", "pilot", "peers"):
        stage_request(mode)
    for script in ("native-install.sh", "adapter/native-run"):
        subprocess.run(["bash", "-n", HERE / script], check=True)
    with tarfile.open(BUNDLE) as tar:
        for path, digest in doc["portable_files"].items():
            assert hashlib.sha256(tar.extractfile(path).read()).hexdigest() == digest, path
    where = remote(doc)
    proof = dict(artifact=dict(version=doc["revision"], files={"mesh": "3" * 64}), installed={
        str(where / path): "4" * 64 for path in (
            *doc["adapter_files"], *doc["sql_files"], "pg17_lock_exact.so", "artifact/mesh.json")})
    for mode in ("pilot", "peers"):
        config = inventory(doc, proof, mode)
        matrix.validate(config)
        for peer in PEERS:
            if peer in config["lanes"]:
                assert config["lanes"][peer] == old["lanes"][peer]
        for block in orders(config, mode):
            assert set(block["order"]) == set(config["lanes"])
        if mode == "pilot":
            assert config["lanes"][CONTROL[0]] == config["lanes"][CONTROL[1]]
        control = deepcopy(config["lanes"][CONTROL[0]]["inventory"]["engines"]["mesh"])
        candidate = config["lanes"][TREATMENT]["inventory"]["engines"]["mesh"]
        index = control["argv"].index("PRODUCT_CLAIM_MODE=stock")
        control["argv"][index] = "PRODUCT_CLAIM_MODE=prepared"
        for path in (*doc["sql_files"], "pg17_lock_exact.so"):
            control["verified_files"][str(where / path)] = proof["installed"][str(where / path)]
        assert candidate == control
    print("Sealed source, recovered FILE broker, duplicate controls, and all three peer inventories verified")


if __name__ == "__main__":
    command, *arguments = sys.argv[1:]
    doc = packet()
    if command == "check":
        check(doc)
    elif command == "setup":
        setup(doc)
    elif command in ("pilot", "peers"):
        sys.exit(timing(doc, command))
    elif command == "replay":
        mode, = arguments
        print(json.dumps(replay(doc, bound_setup(doc), mode), indent=2, sort_keys=True))
    else:
        sys.exit("expected check, setup, pilot, peers, or replay MODE")
