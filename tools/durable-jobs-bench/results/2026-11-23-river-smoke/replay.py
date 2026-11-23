#!/usr/bin/env python3
"""Replay the archived River smokes without PostgreSQL, Go, or network access."""

import hashlib
import json
from pathlib import Path
import sys
import tarfile
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent


def check(condition, message):
    if not condition:
        raise ValueError(message)


def check_sums(manifest, base):
    for line in manifest.read_text().splitlines():
        digest, name = line.split("  ", 1)
        computed = hashlib.sha256((base / name).read_bytes()).hexdigest()
        check(computed == digest, f"checksum mismatch: {name}")


def unpack(name, target):
    with tarfile.open(HERE / name) as archive:
        for member in archive:
            relative = Path(member.name)
            check(not relative.is_absolute() and ".." not in relative.parts, "unsafe archive path")
            if member.isdir():
                continue
            check(member.isfile(), "unexpected archive member")
            path = target / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            with path.open("xb") as stream:
                stream.write(archive.extractfile(member).read())


def negative_controls(short, sparse, verify_run, expected_sha):
    def dup(rows):
        rows.append(rows[0].copy())

    def drop(rows):
        rows.pop()

    def set_to(field, value):
        return lambda row: row.update({field: value})

    cases = [
        ("duplicate attempt", short, "measured/ledger.json", ("attempts",), dup, "duplicate attempt"),
        ("missing attempt", short, "measured/ledger.json", ("attempts",), drop, "missing or substituted attempt"),
        ("duplicate effect", short, "measured/ledger.json", ("effects",), dup, "duplicate effect"),
        ("missing effect", short, "measured/ledger.json", ("effects",), drop, "missing or substituted effect"),
        ("duplicate product", short, "measured/ledger.json", ("product",), dup, "duplicate product"),
        ("missing product", short, "measured/ledger.json", ("product",), drop, "missing or substituted product"),
        ("substituted effect key", short, "measured/ledger.json", ("effects", 0),
         set_to("effect_key", "unknown"), "substituted effect identity"),
        ("substituted attempt key", short, "measured/ledger.json", ("attempts", 0),
         set_to("job_key", "unknown"), "missing or substituted attempt"),
        ("substituted payload hash", short, "measured/ledger.json", ("jobs", 0),
         set_to("payload_sha256", "0" * 64), "wrong job ID or arguments"),
        ("reused write transaction", short, "measured/ledger.json", (),
         lambda rows: rows["effects"][0].update(xid=rows["attempts"][1]["xid"]), "reused producer or application"),
        ("terminal shares application XID", short, "measured/ledger.json", (),
         lambda rows: rows["jobs"][0].update(settled_xid=rows["attempts"][0]["xid"]), "terminal XID reused"),
        ("terminal batch with conflicting timestamps", short, "measured/ledger.json", (),
         lambda rows: rows["jobs"][0].update(settled_xid=next(
             row["settled_xid"] for row in rows["jobs"] if row["settled"] != rows["jobs"][0]["settled"])),
         "batched terminal XID"),
        ("rollback leaked product", sparse, "faults/ledger.json", ("product",),
         lambda rows: rows.append(dict(rows[0], request_id="rollback")), "rollback leaked"),
        ("missing retry attempt", sparse, "faults/ledger.json", ("attempts",), drop, "missing or substituted attempt"),
        ("handler slot exceeded", short, "measured/runner-0-capacity.json", (),
         set_to("peak_concurrent_handlers", 17), "slot cap or handler calls"),
        ("wrong PostgreSQL version", short, "database.json", (),
         set_to("server_version_num", 170010), "PostgreSQL 17.11"),
        ("wrong Go version", short, "config.json", ("build",),
         set_to("GoVersion", "go1.26.3"), "build versions or module sums"),
        ("wrong River version", short, "config.json", ("build", "Deps"),
         lambda deps: next(d for d in deps if d["Path"] == "github.com/riverqueue/river").update(Version="v0.51.1"),
         "build versions or module sums"),
        ("wrong pgx version", short, "config.json", ("build", "Deps"),
         lambda deps: next(d for d in deps if d["Path"] == "github.com/jackc/pgx/v5").update(Version="v5.12.0"),
         "build versions or module sums"),
        ("wrong module sum", short, "config.json", ("build", "Deps", 0),
         set_to("Sum", "h1:changed"), "build versions or module sums"),
        ("wrong binary digest", short, "config.json", (),
         set_to("binary_sha256", "0" * 64), "executed River artifact"),
    ]
    passed = []
    for label, raw, filename, keys, mutate, reason in cases:
        path = raw / filename
        original = path.read_bytes()
        data = json.loads(original)
        target = data
        for key in keys:
            target = target[key]
        mutate(target)
        path.write_text(json.dumps(data))
        try:
            using_sparse = raw == sparse
            verify_run(raw, "sparse" if using_sparse else "short", expected_sha, smoke=using_sparse)
        except ValueError as error:
            check(reason in str(error), f"{label}: rejected for the wrong reason: {error}")
            passed.append(label)
        else:
            raise ValueError(f"accepted invalid evidence: {label}")
        finally:
            path.write_bytes(original)
    return passed


def replay(work):
    check_sums(HERE / "SHA256SUMS", HERE)
    manifest = json.loads((HERE / "manifest.json").read_text())
    source = work / "source"
    original = work / "original"
    unpack("source-verified.tar.gz", source)
    unpack("source-original.tar.gz", original)
    check_sums(source / "SHA256SUMS", source)
    check_sums(HERE / "source-original.SHA256SUMS", original)
    source_go = source / "tools/durable-jobs-bench/go"
    original_go = original / "tools/durable-jobs-bench/go"
    for name in ("db.go", "evidence.go", "main.go", "worker.go", "go.mod", "go.sum", "run", "prepare_input.py"):
        check((source_go / name).read_bytes() == (original_go / name).read_bytes(), f"changed tested source: {name}")

    sys.path[:0] = [str(source_go), str(source_go.parent)]
    from raw_evidence import frozen_input, read_json, verify_run
    from verify import reconcile

    linux = manifest["binary_sha256"]["linux-amd64"]
    local = manifest["binary_sha256"]["darwin-arm64"]
    native, local_results, local_wrapper = {}, {}, {}
    for run in manifest["native"]:
        root = work / run["label"]
        name = run["label"] + ".tar.gz"
        check(run["exit_code"] == 0 and hashlib.sha256((HERE / name).read_bytes()).hexdigest() == run["archive_sha256"],
              f"{name}: native receipt does not match the capture")
        unpack(name, root)
        raw = root / "river"
        ledger, summary = verify_run(raw, run["profile"], linux, smoke=run["faults"])
        result = reconcile(ledger, "go", run["profile"], {
            "version": manifest["version"], "artifact_sha256": linux})
        check(read_json(root / "input.json") == frozen_input(run["profile"])[1], "native input manifest changed")
        check(read_json(root / "runtime.json") == {
            "version": manifest["version"], "artifact_sha256": linux, "phase": "diagnostic"}, "native runtime mismatch")
        if not run["faults"]:
            check(read_json(root / "ledger.json") == ledger, "native export differs from raw reconciliation")
            check(read_json(root / "metrics.json") == result, "native matrix metrics differ from raw reconciliation")
        native[run["label"]] = summary

    unpack("local-raw.tar.gz", work / "local")
    for raw in sorted((work / "local").glob("adapter-*/river")):
        profile = raw.parent.name.split("-")[1]
        check(read_json(raw.parent / "input.json") == frozen_input(profile)[1], "local input manifest changed")
        check(read_json(raw.parent / "runtime.json") == {
            "version": manifest["version"], "artifact_sha256": local, "phase": "diagnostic"}, "local runtime mismatch")
        _, summary = verify_run(raw, profile, local, smoke=True)
        if raw.parent.name == f"adapter-{profile}-01":
            local_results[profile] = summary
        else:
            local_wrapper[raw.parent.name] = summary
    check(local_results.keys() == {"short", "sparse", "noop", "slow", "large"}, "missing local profiles")
    legacy = {}
    for raw in sorted((work / "local").glob("local-*/evidence")):
        legacy[raw.parent.name] = {"binary_sha256": read_json(raw / "config.json")["binary_sha256"],
                                  "original_status": read_json(raw / "status.json")}
    rejected = negative_controls(work / "native-short-01/river", work / "native-sparse-01/river", verify_run, linux)
    return {"native": native, "local": local_results, "local_wrapper": local_wrapper,
            "legacy": legacy, "mutations_rejected": rejected}


if __name__ == "__main__":
    with TemporaryDirectory(prefix="river-smoke-replay-") as temp:
        print(json.dumps(replay(Path(temp)), indent=2, sort_keys=True))
