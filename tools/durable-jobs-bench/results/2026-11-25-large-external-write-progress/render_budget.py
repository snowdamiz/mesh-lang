"""Price changes to eager rendering against complete per-runner LARGE ledgers."""
from collections import defaultdict
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import subprocess
import tarfile

from replay import replay as baseline

ROOT = Path(__file__).resolve().parents[4]
COMMIT = "39f7fae8000c631ea57a7879df21b7b1685fbc4c"
PILOT = "tools/durable-jobs-bench/results/2026-11-25-large-external-factorial/evidence/pilot"


def blob(path, digest):
    data = subprocess.check_output(["git", "show", f"{COMMIT}:{path}"], cwd=ROOT)
    assert sha256(data).hexdigest() == digest
    return data


def deadline_row(owners, deadline):
    effects = [max(j["effect"] for j in rows) for rows in owners.values()]
    terminals = [max(j["terminal"] for j in rows) for rows in owners.values()]
    # Signed. A negative difference means the historical deadline was already met.
    return dict(deadline_us=deadline,
                last_effect_minus_deadline_us=max(effects) - deadline,
                drain_minus_deadline_us=max(terminals) - deadline,
                runners_with_late_effect=sum(t > deadline for t in effects),
                runners_with_late_terminal=sum(t > deadline for t in terminals),
                late_effects=sum(j["effect"] > deadline for rows in owners.values() for j in rows),
                late_terminals=sum(j["terminal"] > deadline for rows in owners.values() for j in rows))


def measure(group, paths, raw_drain):
    digest = group["raw_sha256"]["measured"]["both"]
    raw, = [p for p in paths if f"/{group['group']:02d}-large." in p and
            p.endswith("/measured-both/measured-mesh/raw.tar.gz")]
    with tarfile.open(fileobj=BytesIO(blob(raw, digest))) as archive:
        def read(name):
            return json.load(archive.extractfile(f"adapter/{name}.json"))

        ledger = read("ledger")
        policy = read("queue-policy")
        txn, = ledger["transactions"]
        zero = txn["committed_us"]
        jobs = ledger["jobs"]
        assert txn["enqueued_ids"] == {j["job_key"]: j["id"] for j in jobs}
        assert len(jobs) == len({j["id"] for j in jobs}) == 64
        assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid", "settled_xid")}) == 192
        owners = defaultdict(list)
        for j in jobs:
            assert j["state"] == "completed" and j["effects"] == j["attempts"] == j["hold_ms"] == 1
            assert zero <= j["claimed_us"] <= j["entered_us"] <= j["attempt_committed_us"] <= j["effect_committed_us"] <= j["settled_us"]
            owners[j["worker_pid"]].append(dict(effect=j["effect_committed_us"] - zero,
                                                terminal=j["settled_us"] - zero))
        _, *identities = read("process-identity")
        assert len(owners) == len(identities) == 8
        for i, identity in enumerate(identities):
            ready, report = archive.extractfile(f"adapter/runner-{i}.log").read().splitlines()
            report = json.loads(report)
            assert ready == b"READY" and report["last_error"] is None and report["drained"]
            assert report["batch_protocol"] == "locked_prepare_pg17_v1"
            assert len(owners[identity["pid"]]) == report["claimed"] == report["finished"]
        prepare, = [q for q in policy["queue_statements"] if "prepare_batch_v1(" in q["query"]]
        assert prepare["rows"] == 64
    by_last_effect = sorted(max(j["effect"] for j in rows) for rows in owners.values())
    by_terminal = sorted(max(j["terminal"] for j in rows) for rows in owners.values())
    arm = group["arms"]["both"]
    assert by_last_effect[-1] == round(arm["landmarks"]["effect64_from_commit_ms"] * 1000)
    assert by_terminal[-1] == round(arm["drain_ms"] * 1000)
    return dict(raw_sha256=digest, jobs_per_runner=sorted(map(len, owners.values())),
                prepare_calls=prepare["calls"], aggregate_prepare_ms=round(prepare["execution_ms"], 3),
                independent_runner_drain_floors_us=list(reversed(by_terminal)),
                independent_runner_last_effect_floors_us=list(reversed(by_last_effect)),
                deadlines={str(i): {"tie": deadline_row(owners, t),
                                    "twenty_percent_throughput_lead": deadline_row(owners, t * 5 // 6)}
                           for i, t in raw_drain.items()})


def replay():
    release = json.loads(blob(f"{PILOT}/controller-release.json",
                              "89252dc00d74985d5362dfee823041f39f3079e6a7378d57d83dae314683d5b8"))
    score = json.loads(blob(f"{PILOT}/scored-before-release.json",
                            "ff2f7f59835cd8767309d2f8c54636647c21e6898675fb84d40f10691be25a13"))
    assert release["exit_status"] == 0 and score["advanced"] is False
    assert [g["gates"]["passed"] for g in score["groups"]] == [True, False]
    paths = subprocess.check_output(["git", "ls-tree", "-r", "--name-only", COMMIT, PILOT], cwd=ROOT, text=True).splitlines()
    stock = baseline()["blocks"]
    oban = {n: lanes["oban"]["submit_us"] - lanes["oban"]["stages_us"]["enqueue"] for n, lanes in stock.items()}
    return [measure(g, paths, oban) for g in score["groups"]]


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2))
