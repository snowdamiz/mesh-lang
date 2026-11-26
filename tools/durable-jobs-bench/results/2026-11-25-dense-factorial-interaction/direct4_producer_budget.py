"""Reconcile producer clocks and price complete wake removal against the Direct4 submit target."""
import argparse
import hashlib
import io
import json
import subprocess
import tarfile
from pathlib import Path

from direct4_handler_coverage import control_blocks
from direct4_peer_deadlines import ledger
from direct4_throughput_frontier import replay as throughput_frontier
from warm_river_occupancy import SERIES


DENSE_INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"


def git_bytes(ref, directory, name):
    return subprocess.check_output(["git", "show", f"{ref}:tools/durable-jobs-bench/results/{directory}/{name}"])


def writer_clocks():
    directory = "2026-11-25-dense-producer-layout/evidence"
    score = git_bytes("402459f7b", directory, "scored-before-release.json")
    assert hashlib.sha256(score).hexdigest() == "53fff239f43cbfdb1a4f774dd1d44be8a1dbbd72ac0c97e20b893e69b9d7226a"
    report = json.loads(score)
    assert not report["observer_qualified"]
    rows = []
    for group, block in enumerate(report["groups"], 1):
        arm = block["arms"][1]["measured"]
        raw = git_bytes("402459f7b", directory, f"{group:02}-2-on-measured/measured-mesh/raw.tar.gz")
        with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
            assert hashlib.sha256(raw).hexdigest() == arm["raw_sha256"]
            stats = json.load(archive.extractfile("adapter/producer-attribution.json"))
            jobs = json.load(archive.extractfile("adapter/ledger.json"))
        assert (jobs["runners"], jobs["slots"], len(jobs["jobs"])) == (8, 64, 1024)
        functions = {r["name"]: r for r in stats["functions"]}
        wake = functions["publish_wake_v1"]
        guard = functions["enqueue_batch_layout_v1"]
        assert (wake["calls"], guard["calls"], functions["enqueue_batch_v1"]["calls"]) == (1024, 8, 8)
        assert (wake["total_ms"], guard["total_ms"]) == (arm["function_breakdown"]["wake_ms"],
                                                          arm["function_breakdown"]["layout_ms"])
        rows.append(dict(wake_ms=wake["total_ms"], guard_ms=guard["total_ms"]))
    return rows


def common_field_savings():
    directory = "2026-11-25-dense-common-fields-pilot/setup-release"
    raw = git_bytes("372d29ba3", directory, "raw.tar.gz")
    assert hashlib.sha256(raw).hexdigest() == "9da2f1353ae4d0d0d4ad0c0ea0d41cfac9f4f0824d373ce6c1f4f745cde4fdf6"
    result = []
    with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
        for group in (1, 2):
            clocks = {}
            for lane in ("stock-a", "stock-b", "common"):
                arm = json.load(archive.extractfile(f"./proof/{group}-{lane}/dense.json"))
                elapsed = arm["commit_epoch_us"] * 1000 - arm["submit_epoch_ns"]
                assert abs(arm["submit_to_commit_ms"] * 1e6 - elapsed) < 1
                clocks[lane] = elapsed
            result.append((min(clocks["stock-a"], clocks["stock-b"]) - clocks["common"]) / 1e6)
    return result


def mesh_cpu(exposure):
    result = {}
    for name, block, read, scores, _ in control_blocks(exposure):
        data = read(f"{block}/measured-direct4/measured-mesh/raw.tar.gz")
        jobs, txn = ledger(data, scores["direct4"]["raw_sha256"], topology=(2, 64))
        with tarfile.open(fileobj=io.BytesIO(data)) as archive:
            def get(file):
                return json.load(archive.extractfile("adapter/" + file + ".json"))

            bracket = get("pg-cpu-bracket")
            roster = get("postgres-tls-sessions")
            producer, = [s for s in roster if s["role"] == "parity_producer"]
            assert producer["tls"] and bracket["clock_ticks"] == 100
            writer = get("producer-writer")
            assert (writer["enabled"], writer["bulk"], writer["scalar"]) == (True, 8, 0)
            assert get("ledger")["input_sha256"] == DENSE_INPUT
            identity = get("process-identity")
            assert len(identity) == 3 and all(record["executable_sha256"] ==
                "e836b79a2dbd5ee0e10897de8d3d07fbf789ea038e122d2403799b9879bbbaf4" for record in identity)
            output = archive.extractfile("adapter/broker-output.log").read().splitlines()
            # The producer and observer share the same session. Count observations
            # in the transcript following the enqueue receipt before pricing CPU.
            records = [json.loads(line) for line in output if line.startswith(b"{")]
            receipt, = [i for i, r in enumerate(records) if r.get("xid") == txn["xid"] and "ids" in r]
            assert records[receipt]["ids"] == txn["enqueued_ids"]
            observations = sum("connections" in r and "executing" in r for r in records[receipt+1:])
            assert observations > 0

        first, last = bracket["before"], bracket["after"]
        assert first["end_us"] <= txn["submitted_us"] < txn["committed_us"] <= txn["acknowledged_us"]
        assert max(j["settled_us"] for j in jobs) <= last["begin_us"]
        start, stop = (frame["rows"][str(producer["backend"])] for frame in (first, last))
        assert start[:2] == stop[:2] and start[0] == bracket["postmaster"]
        result[name] = dict(enqueue_us=txn["committed_us"] - txn["submitted_us"],
                            producer_and_observer_cpu_ms=10*(stop[2] - start[2]),
                            observations_after_receipt=observations,
                            after_ack_us=last["begin_us"] - txn["acknowledged_us"])
    return result


def river_clocks():
    scores = json.loads((SERIES / "replay-rest.json").read_bytes())["groups"]
    rows = []
    for group in range(1, 7):
        raw, = SERIES.glob(f"blocks/{group:02}-*/measured-river-warm/*/raw.tar.gz")
        data = raw.read_bytes()
        _, txn = ledger(data, scores[str(group)]["measured"]["river-warm"]["raw_sha256"], topology=(2, 64))
        with tarfile.open(fileobj=io.BytesIO(data)) as archive:
            assert json.load(archive.extractfile("adapter/ledger.json"))["input_sha256"] == DENSE_INPUT
            receipt = json.load(archive.extractfile("adapter/river/measured/enqueue.jsonl"))
        assert receipt["ids"] == txn["enqueued_ids"]
        assert all(receipt[k] == txn[k] for k in ("xid", "read_finished_us", "decoded_us", "enqueue_called_us"))
        stamps = [txn[k] for k in ("submitted_us", "read_finished_us", "decoded_us", "enqueue_called_us",
                                    "committed_us", "returned_us")]
        assert stamps == sorted(stamps) and txn["input_bytes"] == 1158216
        rows.append(dict(read_us=stamps[1]-stamps[0], decode_us=stamps[2]-stamps[1],
                         call_to_commit_us=stamps[4]-stamps[3], after_commit_us=stamps[5]-stamps[4]))
    return rows


def replay(exposure):
    frontier = throughput_frontier(exposure)
    deficits = [row["required_enqueue_saving_us"] / 1000 for row in frontier]
    functions, common = writer_clocks(), common_field_savings()
    wake = max(r["wake_ms"] for r in functions)
    return dict(direct4=mesh_cpu(exposure), warm_river=river_clocks(), observed_8x8=functions,
                common_fields_diagnostic_savings_ms=common,
                remaining_after_full_wake_credit_ms=[min(deficits)-wake, max(deficits)-wake],
                remaining_after_wake_and_common_credit_ms=[min(deficits)-wake-max(common),
                                                           max(deficits)-wake-max(common)])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exposure", type=Path)
    print(json.dumps(replay(parser.parse_args().exposure), indent=2))
