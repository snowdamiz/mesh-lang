#!/usr/bin/env python3
"""Offline release replay using Git-backed bytes. Never launch the benchmark."""
import gzip
import hashlib
import io
import json
from datetime import datetime
from pathlib import Path
from statistics import mean, median
import tarfile

import budgets
import rotation

HERE = Path(__file__).resolve().parent
CASES = ("short", "large")
ATTEMPTS = {
    "setup-de98a0588ecd": (1, "enqueue-spec-preparation"),
    "setup-b0c245fbec03": (0, "enqueue-spec-preparation-mount-retry"),
    "timing-be34d253e19e": (0, "enqueue-spec-preparation-component"),
}
SEALS = {
    "request.json": "5f9110eefc0de20c2bfa7ab58a06d2fe84945182171e66cad206341d818dd156",
    "request-mount-retry.json": "225359096fac2af87b1e1542a5739055732bf8ed4e8eb2a8e5cdd3b4d7e70edc",
}


def sha(contents):
    return hashlib.sha256(contents).hexdigest()


def read(path):
    return json.loads(path.read_bytes())


def verify_members(folder):
    for line in (folder / "SHA256SUMS").read_text().splitlines():
        checksum, relative = line.split("  ", 1)
        assert rotation.digest(folder / relative) == checksum, relative


def released():
    receipts = {}
    for name, (exit_status, request_id) in ATTEMPTS.items():
        folder = HERE / "evidence" / name
        verify_members(folder)
        release = read(folder / "controller-release.json")
        status = read(folder / "status.json")
        assert release["id"] == request_id and status["exit_status"] == release["exit_status"] == exit_status
        assert datetime.fromisoformat(release["started"]) <= datetime.fromisoformat(status["started"])
        assert datetime.fromisoformat(status["finished"]) <= datetime.fromisoformat(release["released_at"])
        raw = folder / "raw.tar.gz"
        assert rotation.digest(raw) == status["raw_sha256"]
        if exit_status:
            assert gzip.decompress(raw.read_bytes()) == b""
            assert not (folder / "dispatch.log").stat().st_size
            assert "No such file or directory" in (folder / "export.log").read_text()
        else:
            with tarfile.open(raw, "r:gz") as archive:
                archived = {Path(m.name).as_posix(): m for m in archive if m.isfile()}
                extracted = {p.relative_to(folder / "raw").as_posix(): p for p in (folder / "raw").rglob("*")
                             if p.is_file()}
                assert archived.keys() == extracted.keys()
                for path, file in extracted.items():
                    assert sha(archive.extractfile(archived[path]).read()) == rotation.digest(file), path
        receipts[name] = {"exit_status": exit_status, "released_at": release["released_at"],
                          "raw_sha256": status["raw_sha256"]}
    setup = HERE / "evidence/setup-b0c245fbec03"
    timing = HERE / "evidence/timing-be34d253e19e"
    assert read(timing / "status.json")["setup_id"] == read(setup / "status.json")["setup_id"]
    assert read(timing / "controller-release.json")["groups"] == list(range(1, 7))
    assert (setup / "raw/SETUP-PASS").is_file()
    assert (setup / "raw/build-pins.json").read_bytes() == (timing / "raw/build-pins.json").read_bytes()
    return receipts, setup / "raw", timing / "raw"


def workloads(request, setup, timing):
    source = HERE / "native-amd64-source.tar.gz"
    assert rotation.digest(source) == request["source_archive_sha256"]
    assert rotation.digest(HERE / "rotation.py") == request["pins"][
        f"tools/durable-jobs-bench/results/{HERE.name}/rotation.py"]
    pins = read(setup / "build-pins.json")
    assert pins["short"]["component_sha256"] != pins["large"]["component_sha256"]
    with tarfile.open(source, "r:gz") as archive:
        def member(name):
            return archive.extractfile(name).read()

        assert member("rotation.py") == (HERE / "rotation.py").read_bytes()
        assert member("probe.rs") == (HERE / "probe.rs").read_bytes()
        generator = {}
        exec(compile(member("fixture-generator.py"), "frozen fixture generator", "exec"), generator)
        settings = json.loads(member("fixture-profiles.json"))
        reports = {}
        for profile in CASES:
            short = profile == "short"
            capture = setup / f"verify/capture-{profile}/capture.json"
            assert rotation.digest(capture) == pins[profile]["capture_verification_sha256"]
            native = read(capture)
            frozen = json.loads(member(f"{profile}-capture.json"))
            for field in ("frames", "jobs", "collector_sha256", "broker_source_sha256",
                          "raw_ledger_archive_sha256", "fixture_transport_sha256"):
                assert native[field] == frozen[field], (profile, field)
            assert sha(member(f"{profile}-broker.mpl")) == native["broker_source_sha256"]
            identity = request["candidate_source_identity"][profile]
            assert sha(member(f"candidate/{profile}/store.rs")) == identity["candidate_store_sha256"]
            assert sha(member(f"{profile}.patch")) == identity["production_patch_sha256"]
            assert sha(member(f"tree-{profile}/compiler/mesh-rt/src/jobs/store.rs")) == identity["stock_store_sha256"]
            assert pins[profile]["lock_sha256"] == sha(member(f"tree-{profile}/Cargo.lock"))
            binary = setup / f"bin/{profile}"
            assert pins[profile]["component_sha256"] == rotation.digest(binary)
            with binary.open("rb") as image:
                header = image.read(20)
            assert header[:6] == b"\x7fELF\x02\x01" and int.from_bytes(header[18:20], "little") == 62
            assert pins[profile]["setup_verification_sha256"] == sha(member(f"{profile}-semantic.json"))
            assert member(f"{profile}-semantic.json") == (setup / f"verify/{profile}.json").read_bytes()
            assert member(f"{profile}-semantic.json") == (timing / f"{profile}-semantic.json").read_bytes()

            count, frame_count, frame_bytes, payload_bytes, slots = (
                (128, 1, 154241, 1024, 16) if short else (64, 64, 524470, 524288, 64))
            assert native["jobs"] == count and len(native["frames"]) == frame_count
            batch, = generator["inputs"](settings["workloads"][profile], slots, settings["seed"])
            transport = json.dumps(batch, indent=2, sort_keys=True, allow_nan=False)
            if short:
                transport = json.dumps({"verb": "enqueue", "batch": json.loads(transport)}, separators=(",", ":"))
            assert sha((transport + "\n").encode()) == native["fixture_transport_sha256"]

            ledger_archive = member(f"{profile}-raw.tar.gz")
            assert sha(ledger_archive) == native["raw_ledger_archive_sha256"]
            with tarfile.open(fileobj=io.BytesIO(ledger_archive), mode="r:gz") as ledger_file:
                ledger = json.load(ledger_file.extractfile("adapter/ledger.json"))
            transaction, = ledger["transactions"]
            assert ledger["profile"] == profile and ledger["slots"] == slots and ledger["phase"] == "measured"
            assert transaction["committed_us"] >= transaction["submitted_us"]
            assert {row["id"] for row in ledger["jobs"]} == set(transaction["enqueued_ids"].values())
            durable_hashes = {row["job_key"]: row["payload_sha256"] for row in ledger["jobs"]}
            jobs = []
            for index, expected in enumerate(native["frames"]):
                frame = member(f"frames/{profile}/{index}")
                specs = json.loads(frame)
                assert {"bytes": len(frame), "sha256": sha(frame), "jobs": len(specs)} == expected
                assert len(frame) == frame_bytes and len(specs) == count // frame_count
                for spec in specs:
                    job = spec["args"]
                    payload = job["payload"].encode("ascii")
                    assert len(payload) == payload_bytes and sha(payload) == durable_hashes[job["key"]]
                    assert spec == {"worker": "parity", "args": job, "request_key": job["key"],
                                    "max_attempts": 3, "backoff_base_ms": 1000, "backoff_max_ms": 1000}
                    jobs.append(job)
            assert jobs == batch["jobs"] and len(durable_hashes) == len(jobs)
            wire = read(setup / f"native-proof/run/{profile}.json")
            assert wire["postgres"] == "17.11" and wire["bulk_calls"] == (7 if short else 0)
            assert wire["accepted_cross_version_replays"] == 32 and wire["savepoint_recovery"] and wire["outer_rollback"]
            reports[profile] = {"jobs": count, "frames": frame_count, "bytes_per_frame": frame_bytes,
                                "frame_bytes_per_submission": frame_count * frame_bytes,
                                "payload_bytes_per_job": payload_bytes, "payload_bytes_per_submission": count * payload_bytes,
                                "native_component_sha256": pins[profile]["component_sha256"], "wire_proof": wire}
        return reports


def interval(values):
    return {"min": round(min(values), 6), "median": round(median(values), 6), "max": round(max(values), 6)}


def price(scores):
    historical = budgets.build()
    assert historical == read(HERE / "budgets.json")
    pricing = {}
    for name, series in historical.items():
        profile = series["profile"]
        wall = scores[profile]["wall_saving_vs_faster_stock_ms"]
        proxy = median(wall)
        raw_series = read(HERE.parent / series["source"])["rows"][profile]
        rows = []
        for budget, raw in zip(series["rotations"], raw_series, strict=True):
            assert budget["group"] == raw["group"]
            stock_time = mean(budget["stock_submit_to_commit_ms"])
            scores_by_lane = raw["scores"]
            peer_gaps = {}
            controls = ("bulk-d2",) if profile == "short" else ("ordinary-a", "ordinary-b")
            full_submit = {lane: (row["last_settlement_us"] - row["first_submit_us"]) / 1000
                           for lane, row in scores_by_lane.items()}
            stock_full = mean(full_submit[lane] for lane in controls)
            for peer, old_full_gap in budget["saving_needed_ms"].items():
                full_gap = stock_full - full_submit[peer]
                # The frozen budget rounded a rate-derived value to a whole microsecond.
                assert abs(full_gap - old_full_gap) <= 0.000500001
                enqueue_gap = stock_time - scores_by_lane[peer]["first_submit_to_commit_ms"]
                peer_gaps[peer] = {
                    "submit_to_commit_gap_ms": round(enqueue_gap, 6),
                    "full_submit_gap_ms": round(full_gap, 6),
                    "share_of_positive_submit_to_commit_gap_pct": round(100 * proxy / enqueue_gap, 6) if enqueue_gap > 0 else None,
                    "share_of_positive_full_submit_gap_pct": round(100 * proxy / full_gap, 6) if full_gap > 0 else None,
                }
            rows.append({"group": raw["group"], "stock_submit_to_commit_ms": stock_time,
                         "estimated_share_of_stock_submit_to_commit_pct": round(100 * proxy / stock_time, 6),
                         "peer_gaps": peer_gaps})
        pricing[name] = {"profile": profile, "source_sha256": series["source_sha256"],
                         "wall_saving_proxy_ms_per_submission": round(proxy, 6), "rotations": rows}
    return pricing


def main():
    for path, digest in SEALS.items():
        assert rotation.digest(HERE / path) == digest
    request = read(HERE / "request.json")
    repair = read(HERE / "request-mount-retry.json")
    for field in ("decision", "source_archive_sha256", "candidate_source_identity", "pins"):
        assert repair[field] == request[field]
    for field, value in request["conditional_broker"].items():
        assert repair["conditional_broker"][field] == (
            value.replace("controller.py", "mount-retry.py") if field == "setup_command" else value)
    repair_source = (HERE / "mount-retry.py").read_text()
    assert rotation.digest(HERE / "mount-retry.py") == next(iter(repair["local_sha256"].values()))
    assert 'findmnt -no MAJ:MIN' in repair_source and 'findmnt -nro MAJ:MIN' in repair_source

    receipts, setup, timing = released()
    work = workloads(request, setup, timing)
    summary = rotation.replay(timing)
    assert summary == read(timing / "summary.json")
    components = {}
    for profile in CASES:
        row = summary["profiles"][profile]
        assert row["wins_over_jitter"] == 6 and not row["advance_to_broker_trial"]
        assert len(row["raw_sha256"]) == 24
        components[profile] = {
            "arms_cpu_ms_per_submission": row["arm_cpu_ms"],
            "arms_wall_ms_per_submission": row["arm_wall_ms"],
            "repetitions_per_arm": rotation.REPEATS[profile],
            "cpu_saving_ms_per_submission": interval(row["cpu_saving_vs_faster_stock_ms"]),
            "wall_saving_ms_per_submission": interval(row["wall_saving_vs_faster_stock_ms"]),
            "duplicate_cpu_spread_ms": interval(row["duplicate_cpu_spread_ms"]),
            "diagnostic_stage_wall_ms_per_submission": row["stock_stage_wall_ms_diagnostic_only"],
            "scored_rotations": 6, "raw_rows": len(row["raw_sha256"]),
            "wins_over_jitter": row["wins_over_jitter"], "promotion_cpu_ms": row["minimum_ms"],
            "advance_to_broker_trial": row["advance_to_broker_trial"],
        }
    output = {"release_receipts": receipts, "workloads": work, "components": components,
              "frozen_scorer_sha256": rotation.digest(HERE / "rotation.py"),
              "original_summary_sha256": rotation.digest(timing / "summary.json"),
              "historical_pricing": price(summary["profiles"])}
    print(json.dumps(output, indent=2, sort_keys=True, allow_nan=False))


if __name__ == "__main__":
    main()
