#!/usr/bin/env python3
"""Replay River's timed input boundary from original setup evidence."""

import hashlib
import json
from pathlib import Path
import sys
import tarfile
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_sums(base):
    for line in (base / "SHA256SUMS").read_text().splitlines():
        checksum, name = line.split("  ", 1)
        if digest(base / name) != checksum:
            raise ValueError(f"checksum mismatch: {name}")


def unpack(name, destination):
    with tarfile.open(HERE / name) as archive:
        for member in archive:
            path = Path(member.name)
            if path.is_absolute() or ".." in path.parts:
                raise ValueError(f"unsafe archive path: {path}")
            if member.isdir():
                continue
            if not member.isfile():
                raise ValueError(f"unexpected archive member: {path}")
            target = destination / path
            target.parent.mkdir(parents=True, exist_ok=True)
            with target.open("xb") as stream:
                stream.write(archive.extractfile(member).read())


def replay(work):
    check_sums(HERE)
    ready = json.loads((HERE / "READY.json").read_text())
    for name, checksum in ready["archives"].items():
        if digest(HERE / name) != checksum:
            raise ValueError(f"handoff archive checksum differs: {name}")
    source = work / "source"
    unpack("wrapper-source.tar.gz", source)
    check_sums(source)
    suite = source / "tools/durable-jobs-bench"
    sys.path[:0] = [str(suite / "go"), str(suite)]
    from raw_evidence import epoch_us, frozen_input, read_json, verify_run
    from verify import reconcile, require

    native_hash = ready["artifact_sha256"]
    summaries = {}

    def verify(label, path, profile, binary_hash, *, smoke=False, warmup=False):
        ledger, summary = verify_run(path, profile, binary_hash, smoke=smoke, warmup=warmup)
        transactions = ledger["transactions"]
        summaries[label] = {
            "submitted_batches": len(transactions),
            "submitted_file_bytes": sum(row["input_bytes"] for row in transactions),
            "completed": summary["measured"]["completed"],
            "independent_attempt_rows": summary["measured"]["attempt_rows"],
            "independent_effect_rows": summary["measured"]["effect_rows"],
            "terminal_transactions": summary["measured"]["terminal_transactions"],
            "handler_slots": summary["measured"]["slot_cap"],
            "physical_connection_cap": summary["measured"]["physical_cap"],
        }
        if smoke:
            summaries[label]["rollback_receipts"] = summary["faults"]["rolled_back_batches"]
            summaries[label]["retry_jobs"] = summary["faults"]["retried"]
        if warmup:
            summaries[label]["warmup_completed"] = summary["warmup"]["completed"]
        return ledger

    for profile in ("large", "sparse"):
        native = work / f"native-{profile}"
        unpack(f"native-{profile}.tar.gz", native)
        runtime = read_json(native / "runtime.json")
        require(runtime["phase"] == "diagnostic" and runtime["artifact_sha256"] == native_hash
                and runtime["submit_boundary"] == ready["submit_boundary"], "wrong native run identity")
        config = read_json(native / "river/config.json")
        require(config["os"] == "linux" and config["arch"] == "amd64"
                and config["visible_cpus"] == len(ready["native_setup"]["cpus"]), "wrong native runner topology")
        require(read_json(native / "input.json") == frozen_input(profile)[1], "wrong staged input manifest")
        ledger = verify(f"native-{profile}", native / "river", profile, native_hash, smoke=profile == "sparse")
        if profile == "large":
            require(read_json(native / "ledger.json") == ledger, "native wrapper ledger differs from replay")
            metrics = reconcile(ledger, "go", profile,
                                {"artifact_sha256": native_hash, "version": ready["version"]})
            require(read_json(native / "metrics.json") == metrics, "native matrix metrics differ from replay")

    local = work / "local"
    unpack("local-raw.tar.gz", local)
    local_hash = ready["local_artifact_sha256"]
    verify("local-large", local / "large", "large", local_hash, smoke=True, warmup=True)
    verify("local-sparse", local / "sparse", "sparse", local_hash, smoke=True)
    short = local / "short"
    ledger = verify("local-short", short / "river", "short", local_hash)
    require((short / "run.log").read_text().strip() == "invalid peak runner RSS", "wrong local wrapper failure")
    try:
        reconcile(ledger, "go", "short", {"artifact_sha256": local_hash, "version": ready["version"]})
    except ValueError as error:
        require(str(error) == "invalid peak runner RSS", "unexpected matrix reconciliation failure")
    else:
        raise ValueError("matrix accepted the macOS-only RSS sample")

    # Use one small archive to prove the strict verifier rejects invalid clock/byte evidence.
    raw = work / "native-sparse/river"
    path = raw / "measured/enqueue.jsonl"
    original = path.read_text()
    lines = original.splitlines()
    first = json.loads(lines[0])
    rejected = []
    for change, expected in (
        ({"read_finished_us": first["submitted_us"] - 1}, "invalid batch read, decode or enqueue timestamps"),
        ({"input_bytes": first["input_bytes"] - 1}, "wrong batch byte count"),
        ({"enqueue_called_us": epoch_us(first["commit"]) + 1}, "product did not commit with enqueue"),
    ):
        path.write_text("\n".join([json.dumps(first | change), *lines[1:]]) + "\n")
        try:
            verify_run(raw, "sparse", native_hash, smoke=True)
        except ValueError as error:
            require(expected in str(error), "mutation failed at the wrong check")
            rejected.append(expected)
        else:
            raise ValueError(f"mutation passed: {expected}")
        finally:
            path.write_text(original)
    return {"setup_only": True, "smokes": summaries, "rejected_controls": rejected}


if __name__ == "__main__":
    with TemporaryDirectory(prefix="river-submit-replay-") as location:
        print(json.dumps(replay(Path(location)), indent=2, sort_keys=True))
