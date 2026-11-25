"""Recover the failed SLOW SETUP with exact launch-bound TLS rosters."""

from collections import Counter
from pathlib import Path
import json
import sys
import tarfile

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent
RESULTS = PARENT.parent
sys.path.insert(0, str(PARENT))
import controller as base

from score import BROKER, BROKER_SHA, ELF, RUNNER

sys.path.insert(0, str(HERE / "stage"))
import roster

run = base.run
ROOT, INBOX = base.ROOT, base.INBOX
ID = PARENT.name + "-roster-retry"
PACKET = HERE / "controller-packet.json"
ORIGINAL = PARENT / "evidence/native/setup"
DENSE = RESULTS / "2026-11-25-dense-live-grant-plans/procfd-recovery"
CAP71 = RESULTS / "2026-11-24-ordinary-slow-equal-topology-cap71"
OLD_ID = PARENT.name + "-setup"
OLD_SHA = "7f0cba45fd78c74c35bb05c688b451c85d9fbb2fbffda7fb59d029db9f400465"


def archived_denial():
    parent_packet = PARENT / "controller-packet.json"
    assert run.digest(parent_packet) == OLD_SHA
    fresh = run.read_json(ORIGINAL / "controller-release.json")
    assert (fresh["id"], fresh["exit_status"], fresh["request_sha256"], fresh["released_at"]) == (
        OLD_ID, 1, OLD_SHA, "2026-11-25T04:14:27.587298+00:00")
    assert run.read_json(ORIGINAL / "grant.json") == fresh["grant"]
    series = run.read_json(ORIGINAL / "series.json")
    block, = base.blocks("setup")
    path, = base.matrix.attempt_paths(ORIGINAL, block)
    assert run.read_json(path / "order.json") == block
    arms = {}
    for phase, label, folder in base.files_for(path, block, series["config"]):
        archive = folder / "raw.tar.gz"
        with tarfile.open(archive) as raw:
            def get(name):
                return run.file_json(raw, f"adapter/{name}.json")

            host = run.file_json(raw, "host.json")
            invocation = run.file_json(raw, "invocation.json")
            assert (invocation["profile"], invocation["phase"], invocation["engine"]) == ("slow", phase, "mesh")
            assert run.file_json(raw, "status.json")["exit_code"] == 1
            assert host["verified_files"][RUNNER] == ELF and host["verified_files"][BROKER] == BROKER_SHA
            assert set(raw.getnames()).isdisjoint({"adapter/ledger.json", "adapter/enqueue-batch_000000.log"})
            failure = get("failure")
            processes = get("process-identity")
            broker, *runners = processes
            pid = runners[0]["pid"]
            assert (failure["type"], failure["message"]) == (
                "PermissionError", f"[Errno 13] Permission denied: '/proc/{pid}/fd'")
            assert ["slow.py", 113, "capture_runners"] in failure["stage"]
            assert ["socket_map.py", 24, "runner_backend_map"] in failure["stage"]
            assert broker["argv"] == [BROKER, "session"] and broker["executable_sha256"] == BROKER_SHA
            assert all(r["argv"] == [RUNNER, "run"] and r["executable_sha256"] == ELF for r in runners)
            counts = Counter(row["role"] for row in get("postgres-tls-sessions") if row["tls"])
            assert counts == roster.roles(len(runners), len(runners))
            arms[label] = dict(raw_sha256=run.digest(archive), first_error=failure["type"],
                               failing_frames=failure["stage"][-3:], ready_tls=dict(counts),
                               no_enqueue=True, owned_runner_methods=[r["method"] for r in runners])
    return dict(original_release=fresh["released_at"], exit_status=1, arms=arms,
                original_release_sha256=run.digest(ORIGINAL / "controller-release.json"))


def archived_precedent():
    qualified = run.read_json(DENSE / "evidence/qualified-roster.json")
    for name in ("raw_archive", "release"):
        key = "raw_sha256" if name == "raw_archive" else "release_sha256"
        assert run.digest(ROOT / qualified[name]) == qualified[key]
    dense = DENSE / "evidence/native/blocks"
    good, = dense.glob("01-noop/sampled/measured-mesh/raw.tar.gz")
    early, = dense.glob("02-noop/sampled/measured-mesh/raw.tar.gz")
    with tarfile.open(good) as raw:
        witness = run.file_json(raw, "adapter/grant-roster.json")
        assert [len(witness[k]["sessions"]) for k in ("first", "both")] == [21, 41]
    with tarfile.open(early) as raw:
        counts = Counter(row["role"] for row in run.file_json(raw, "adapter/postgres-tls-sessions.json"))
        assert counts == dict(compiled_caller=32, parity_producer=1, parity_subject=6)
        assert run.file_json(raw, "adapter/failure.json")["type"] == "AssertionError"
    cap71, = (CAP71 / "evidence/native/sql-retry-timing/blocks").glob(
        "01-slow.*/measured-mesh/measured-mesh/raw.tar.gz")
    with tarfile.open(cap71) as raw:
        before, after = [run.file_json(raw, f"adapter/sessions-{boundary}.json")
                         for boundary in ("before", "end")]
        assert Counter(row["role"] for row in before if row["tls"]) == roster.roles(2, 2)
        assert {(r["backend"], r["client_port"], r["role"]) for r in before} == {
            (r["backend"], r["client_port"], r["role"]) for r in after}
        assert run.file_json(raw, "adapter/database.json")["max_connections"] == 71
        topology = run.file_json(raw, "adapter/topology.json")
        assert (topology["runners"], topology["slots_per_runner"], topology["app_sessions_per_runner"]) == (2, 32, 4)
    return dict(dense_serial_launch_sha256=run.digest(good), dense_early_partial_sha256=run.digest(early),
                dense_early_roles=dict(counts), cap71_warm_and_end_sha256=run.digest(cap71))


original_replay = base.replay_block
base.HERE, base.ID = HERE, ID
base.STAGE = sorted((HERE / "stage").iterdir())


def replay_block(path, config, block):
    result = original_replay(path, config, block)
    for phase, label, folder in base.files_for(path, block, config):
        if label not in base.MESH:
            continue
        with tarfile.open(folder / "raw.tar.gz") as raw:
            def get(name):
                return run.file_json(raw, f"adapter/{name}.json")

            _, *runners = get("process-identity")
            pids = [p["pid"] for p in runners]
            witness = get("ready-roster")
            owners = roster.verify(witness, pids)
            assert get("postgres-tls-sessions") == roster.standard(witness["ready"])
            assert owners == get("slow-transfer")["warm_apps_by_runner"]
            transaction, = get("ledger")["transactions"]
            assert witness["ready"]["observed_us"] + 3_000_000 <= witness["before_submit"]["observed_us"]
            assert witness["before_submit"]["observed_us"] < transaction["submitted_us"]
            result[phase][label]["exact_ready_and_before_submit"] = sum(map(len, owners.values()))
    return result


base.replay_block = replay_block


def check():
    doc = run.read_json(PACKET)
    assert doc["id"] == ID and doc["ready"] and doc["build_free"]
    assert doc["original_packet_sha256"] == OLD_SHA
    for name, digest in doc["local_sha256"].items():
        assert run.digest(ROOT / name) == digest, name
    assert doc["original_denial"] == archived_denial()
    assert doc["native_precedent"] == archived_precedent()
    original = run.read_json(PARENT / "controller-packet.json")
    for phase in base.PHASES:
        frozen = original["configuration"][phase]
        actual = doc["configuration"][phase]
        assert actual == base.configure(phase)
        assert doc["schedule"][phase] == original["schedule"][phase] == base.blocks(phase)
        for label, lane in actual["lanes"].items():
            if label not in base.MESH:
                assert lane == frozen["lanes"][label]
                continue
            old = frozen["lanes"][label]["inventory"]["engines"]["mesh"]
            new = lane["inventory"]["engines"]["mesh"]
            assert old["argv"][2:] == new["argv"][2:]
            nonstage = lambda spec: {
                path: digest for path, digest in spec["verified_files"].items()
                if not path.startswith(str(run.REMOTE) + "/")}
            assert nonstage(old) == nonstage(new)
            assert all(old[key] == new[key] for key in old.keys() - {"argv", "verified_files"})
    assert doc["gate"] == original["gate"] and doc["minutes"] == original["minutes"]
    assert (doc["cpus"], doc["cap"], doc["slots"], doc["app_sessions"]) == ([0, 1], 68, 64, 8)
    rows = [json.loads(line) for line in (HERE / "evidence/lifecycle.jsonl").read_text().splitlines()]
    assert [row["roster_sizes"] for row in rows] == [[1, 13, 13], [1, 9, 17, 17]]
    assert [row["libpq_owner_counts"] for row in rows] == [[8], [4, 4]]
    for row in rows:
        assert len(row["refusals"]) == 9 and row["uid"] == 65534 and row["kernel"] == "Linux"
        assert row["fd_denials"] == row["runners"] and row["postgres_version"] == 170011
        assert row["caps"]["max_connections"] == 68 and not row["stage_listable"]
    return doc


if __name__ == "__main__":
    action, *args = sys.argv[1:]
    doc = check()
    if action == "check" and not args:
        print("Verified original four denials, prior native rosters, exact sibling settings and untimed TLS controls.")
    else:
        phase, = args
        if action == "dispatch":
            sys.exit(base.dispatch(phase, doc))
        if action == "replay":
            print(json.dumps(base.report(phase, doc), indent=2, sort_keys=True, allow_nan=False))
        else:
            sys.exit("expected check, dispatch PHASE, or replay PHASE")
