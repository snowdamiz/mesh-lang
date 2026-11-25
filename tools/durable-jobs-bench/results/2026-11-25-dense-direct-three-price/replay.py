"""Price a third synchronous worker against the two released endpoints."""
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[4]
SOURCE = "f9002c6ce0af37202ba044cef6dab2483c2080cd"
PREFIX = "tools/durable-jobs-bench/results/2026-11-25-dense-fixed8-pool-supply/evidence/native"
CLOCKS = ("submit_us", "drain_us", "middle_512_us")


def blob(path):
    return subprocess.check_output(["git", "-C", ROOT, "show", f"{SOURCE}:{PREFIX}/{path}"])


def measured(group, label, original):
    raw = blob(f"blocks/{int(group):02d}-noop/measured-{label}/measured-mesh/raw.tar.gz")
    assert sha256(raw).hexdigest() == original["raw_sha256"]
    with tarfile.open(fileobj=BytesIO(raw)) as archive:
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
    tx, = ledger["transactions"]
    jobs = ledger["jobs"]
    assert ledger["runners"] == 2 and ledger["slots"] == 64
    assert len(jobs) == len({j["job_key"] for j in jobs}) == 1024
    assert len({j[key] for j in jobs for key in ("attempt_xid", "effect_xid")}) == 2048
    assert all(j["state"] == "completed" and j["attempts"] == j["effects"] == 1 for j in jobs)
    effects = sorted(j["effect_committed_us"] for j in jobs)
    end = max(j["settled_us"] for j in jobs)
    clocks = dict(submit_us=end - tx["submitted_us"], drain_us=end - tx["committed_us"],
                  middle_512_us=effects[767] - effects[255])
    assert all(clocks[key] == original[key] for key in CLOCKS)
    cpu_us = round(ledger["runner_cpu_seconds"] * 1_000_000)
    assert cpu_us == round(original["runner_cpu_ms"] * 1000)
    return dict(**clocks, cpu_us=cpu_us, raw_sha256=original["raw_sha256"])


def replay():
    score = json.loads(blob("pilot-replay.json"))
    release = blob("controller/pilot-release.json")
    assert sha256(release).hexdigest() == score["release_sha256"]
    assert json.loads(release)["request_sha256"] == score["timing_request_sha256"]
    assert json.loads(release)["exit_status"] == 0 and score["gate"]["passing"] == 0
    groups = {}
    for group, rows in score["measured"].items():
        arms = {label: measured(group, label, original) for label, original in rows.items()}
        two, four = arms["direct2"], arms["direct4"]
        controls = [arms["service2-a"], arms["service2-b"]]
        cpu_limit = max(c["cpu_us"] for c in controls)
        room = cpu_limit - two["cpu_us"]
        cost = four["cpu_us"] - two["cpu_us"]
        assert 0 < room < cost
        bounds = {}
        for key in CLOCKS:
            gate = score["gate"]["blocks"][group]["clocks"][key]
            assert min(c[key] for c in controls) == gate["control_us"]
            deadline = gate["control_us"] - gate["threshold_us"]
            gain = two[key] - four[key]
            assert gain > 0
            bounds[key] = dict(deadline_us=deadline, required_advance_from_two_us=two[key] - deadline,
                               measured_two_to_four_gain_us=gain,
                               required_fraction=(two[key] - deadline) / gain)
        groups[group] = dict(arms=arms, bounds=bounds, max_runner_cpu_us=cpu_limit,
                             headroom_over_two_us=room, extra_cpu_of_four_us=cost,
                             affordable_fraction=room / cost)
    return dict(source=SOURCE, release_sha256=score["release_sha256"], groups=groups)


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
