"""Reconcile actual FILE producer and claim pilots before testing them together."""
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[4]
RESULTS = "tools/durable-jobs-bench/results"
PRODUCER = ("ab6ba68a1f27df13b3337c2f64d7074a6ee1ccd4",
            f"{RESULTS}/2026-11-25-dense-one-batch-lookahead/native/evidence/pilot")
CLAIM = ("bb16edc3a61ee0a45e3f06ccffaac444592c1144",
         f"{RESULTS}/2026-11-25-public-large-exact-prepare/evidence/native-pilot")


def git(*args):
    return subprocess.check_output(["git", "-C", str(ROOT), *args])


def blob(series, path):
    revision, directory = series
    return git("show", f"{revision}:{directory}/{path}")


def arm(series, path, digest, clocks, last_confirmed):
    raw = blob(series, path)
    assert sha256(raw).hexdigest() == digest
    with tarfile.open(fileobj=BytesIO(raw)) as archive:
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        storage = json.load(archive.extractfile("adapter/storage-topology.json"))
        assert (storage["storage"], storage["args_storage"], storage["topology"]) == ("default", "x", "stock")
        writer = json.load(archive.extractfile("adapter/producer-writer.json"))
        assert writer["enabled"] and (writer["scalar"], writer["bulk"]) == (64, 0)
        final_receipts = []
        if last_confirmed is not None:
            for i in range(8):
                log = archive.extractfile(f"adapter/runner-{i}.log").read().decode().splitlines()
                state = json.loads(log[-1][log[-1].index("{"):])
                assert state["drained"] and state["claimed"] == state["finished"]
                if state["claimed"]:
                    final_receipts.append((state["last_batch_confirmed_us"], state["claimed"]))
            assert sum(count for _, count in final_receipts) == 64
    tx, = ledger["transactions"]
    jobs = ledger["jobs"]
    assert ledger["runners"] == 8 and ledger["slots"] == 64
    assert len(jobs) == len({j["job_key"] for j in jobs}) == 64
    assert tx["enqueued_ids"] == {j["job_key"]: str(j["id"]) for j in jobs}
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 128
    assert len({j["settled_xid"] for j in jobs}) == 64
    for j in jobs:
        assert j["attempt"] == j["attempts"] == j["effects"] == 1 and j["state"] == "completed"
        assert j["entered_us"] <= j["attempt_committed_us"] <= j["effect_committed_us"] <= j["settled_us"]
    submitted, committed = tx["submitted_us"], tx["committed_us"]
    effects = sorted(j["effect_committed_us"] for j in jobs)
    end = max(j["settled_us"] for j in jobs)
    times = {"enqueue": committed - submitted, "drain": end - committed, "submit": end - submitted}
    assert times == clocks
    for n in (8, 32, 56, 64):
        times[f"submit_to_effect_{n}"] = effects[n-1] - submitted
        times[f"commit_to_effect_{n}"] = effects[n-1] - committed
    times["final_effect_to_settlement"] = end - effects[-1]
    receipt = {"runners_with_effects": len({j["worker_pid"] for j in jobs})}
    if final_receipts:
        latest = max(t for t, _ in final_receipts)
        assert latest - committed == last_confirmed
        receipt.update(runners_with_confirmed_grants=len(final_receipts),
                       last_confirmation_after_commit_us=last_confirmed,
                       effects_committed_at_last_confirmation=sum(t <= latest for t in effects),
                       last_confirmation_to_final_effect_us=effects[-1] - latest,
                       claims_per_productive_runner=sorted(count for _, count in final_receipts))
    return times, {j["job_key"]: j["payload_sha256"] for j in jobs}, receipt


def replay_series(series, *, producer):
    release = json.loads(blob(series, "controller-release.json"))
    assert release["exit_status"] == 0
    seal = ("213ed54c69eb117d7ed93cdbc558bc9e8e4ff15653c36518caa6e43c28d64959" if producer else
            "ae6d76cd149dee5f36ddbd34f2efa4c905178054302b4cbe87fc873062d96801")
    assert release["request_sha256"] == seal
    summary = json.loads(blob(series, "scored-before-release.json" if producer else "summary.json"))
    assert (summary["advanced"] if producer else summary["qualifies"]) is False
    revision, directory = series
    blocks = git("ls-tree", "--name-only", f"{revision}:{directory}/blocks").decode().splitlines()
    output = []
    inputs = None
    for i in range(2):
        if producer:
            group = summary["groups"][i]
            assert group["group"] == i + 1
            scored = group["arms"]
            hashes = group["raw_sha256"]["measured"]
            controls, treatment = ("off-a", "off-b"), "on"
        else:
            scored = summary["scores"][i]
            hashes = {label: s["raw_sha256"] for label, s in scored.items()}
            controls, treatment = ("control-a", "control-b"), "prepared"
        block, = [b for b in blocks if b.startswith(f"{i+1:02}-")]
        measured, receipts = {}, {}
        for label, scores in scored.items():
            unit = "ms" if producer else "us"
            clocks = {k: round(scores[f"{k}_{unit}"] * (1000 if producer else 1)) for k in ("submit", "enqueue", "drain")}
            path = f"blocks/{block}/measured-{label}/measured-mesh/raw.tar.gz"
            measured[label], payloads, receipts[label] = arm(
                series, path, hashes[label], clocks, scores.get("last_confirmed_us"))
            if inputs is None:
                inputs = payloads
            assert inputs == payloads
        best = min(controls, key=lambda lane: measured[lane]["submit"])
        savings = {k: v - measured[treatment][k] for k, v in measured[best].items()}
        assert savings["enqueue"] + savings["drain"] == savings["submit"]
        assert savings["submit_to_effect_64"] + savings["final_effect_to_settlement"] == savings["submit"]
        output.append(dict(group=i+1, fastest_full_control=best, savings_vs_that_control_us=savings,
                           clocks_us=measured, receipts=receipts, raw_sha256=hashes))
    return output, inputs


if __name__ == "__main__":
    producer, p = replay_series(PRODUCER, producer=True)
    claim, c = replay_series(CLAIM, producer=False)
    assert p == c
    print(json.dumps(dict(producer=producer, claim=claim), indent=2, sort_keys=True))
