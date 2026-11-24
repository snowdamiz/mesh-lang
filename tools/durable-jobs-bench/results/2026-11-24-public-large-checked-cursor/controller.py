"""Grant-bound, build-free native price of large speculative admission."""
import argparse
from copy import deepcopy
from datetime import datetime, timezone
from importlib import import_module
import json
from pathlib import Path
import re
from statistics import median
import subprocess
import sys
import tarfile

from audit import ARCHIVE, ELF, HERE, HOST, INPUT, MANIFEST, PRODUCT, ROOT, audit, digest, member, read, stock_mesh, verify_seal
from trace import analyze

SUITE = PRODUCT.parents[1]
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
STORAGES = {"large": "default", "large-external": "external-4m"}
ORDER = ("stock-a", "thin", "trace", "stock-b")
BLOCKS = [dict(group=i, profile="large", phases=["warmup", "measured"], order=list(order))
          for i, order in enumerate((ORDER, ORDER[::-1]), 1)]
FILES = ("launch.sh", "observe.py", "diagnostic.py")
MATRIX = HERE.parent / "2026-11-23-bulk-enqueue-v2-short/replay"
sys.path[:0] = [str(SUITE), str(MATRIX)]
matrix = import_module("interleave-v2")
run = matrix.run
matrix.ATTEMPT_ERRORS += (AssertionError,)


def request_path(lane):
    return HERE / f"intake-{STORAGES[lane]}.json"


def request_id(lane):
    return f"{HERE.name}-{STORAGES[lane]}"


def output(lane):
    return HERE / "evidence/native" / STORAGES[lane]


def configure(lane):
    original = read(ARCHIVE / f"{lane}.inventory.json")
    cfg = deepcopy(original["configuration"])
    assert cfg["host"] == HOST and cfg["cpus"] == [0, 1]
    assert original["recipe"]["storage"] == STORAGES[lane] and original["recipe"]["input_sha256"] == INPUT
    assert all(b["phases"] == ["warmup", "measured"] for b in original["recipe"]["blocks"])
    product = cfg["lanes"]["product"]
    cfg.update(description=f"Public large checked cursor diagnostic, {STORAGES[lane]}", lanes={})
    for label in ORDER:
        lane_config = deepcopy(product)
        spec = lane_config["inventory"]["engines"]["mesh"]
        assert lane_config["product_mode"] == "product-bulk"
        assert spec["artifact_sha256"] == MANIFEST
        assert {f"PRODUCT_ELF_SHA256={ELF}", f"PRODUCT_MANIFEST_SHA256={MANIFEST}",
                f"PRODUCT_STORAGE={STORAGES[lane]}", "PRODUCT_TOPOLOGY=stock"} <= set(spec["argv"])
        assert spec["argv"][-2].endswith("/replay-closure-retry/native-run")
        if label in ("thin", "trace"):
            stage = run.REMOTE / f"{request_id(lane)}-{label}"
            spec["argv"][-2:-1] = ["/bin/bash", str(stage / "launch.sh"), spec["argv"][-2], label]
            spec["verified_files"].update({str(stage / file): digest(HERE / file) for file in FILES})
        cfg["lanes"][label] = lane_config
    matrix.validate(cfg)
    return cfg


def inputs(lane):
    packet = read(request_path(lane))
    assert packet["id"] == request_id(lane) and packet["schedule"] == BLOCKS and packet["no_build"]
    assert packet["input_sha256"] == INPUT and packet["manifest_sha256"] == MANIFEST
    for path, sha in packet["local_sha256"].items():
        assert digest(ROOT / path) == sha, path
    cfg = configure(lane)
    proof = PRODUCT / "evidence/native/setup/trusted-jobs-product-623c46b92cc5-closure-retry/qualification.json"
    p = read(proof)
    assert (p["manifest_sha256"], p["executable_sha256"]) == (MANIFEST, ELF)
    assert digest(proof.with_name("raw.tar.gz")) == p["raw_sha256"]
    assert all(p["files"].items() <= l["inventory"]["engines"]["mesh"]["verified_files"].items()
               for l in cfg["lanes"].values())
    baseline = read(HERE / "evidence/audit.json")
    assert len(baseline["selected"][lane]) == 5
    assert packet["baseline"] == baseline["large_baseline"][lane]
    return cfg, packet, baseline["selected"][lane]["1"]["payload_sha256"]


def grant(lane):
    packet = request_path(lane)
    id = request_id(lane)
    assert packet.read_bytes() == (QUEUE / f"{id}.json").read_bytes()
    entries = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                         (QUEUE / "STATUS.md").read_text(), re.M)
    kind, key, start, end = entries[-1]
    assert (kind, key) == ("TIMING", id)
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return dict(start=start, end=end, request=digest(packet))


def physical_check(rows, payloads, lane):
    assert len(rows) == len(payloads) == 64
    assert {r["job_key"]: r["payload_sha256"] for r in rows} == payloads
    reasons = {"size_only": 0, "compression_only": 0, "both": 0}
    for row in rows:
        assert row["payload_bytes"] == 524288
        compression = any(row[k] is not None for k in ("args_compression", "metadata_compression", "tags_compression"))
        large = row["size_sum"] > 65536
        assert large or compression
        if lane == "large-external":
            assert row["args_compression"] is None and large
        reasons["both" if large and compression else "size_only" if large else "compression_only"] += 1
    return reasons


def score(folder, phase, label, block, cfg, lane, payloads):
    result = run.score_arm(folder, block, phase, "mesh", cfg["lanes"][label]["inventory"])
    path = folder / "raw.tar.gz"
    result["raw_sha256"] = digest(path)
    observed = stock_mesh(path, result, lane)
    assert observed["payload_sha256"] == payloads
    with tarfile.open(path) as raw:
        assert len(member(raw, "process-identity")) == 9
        tls = member(raw, "postgres-tls-sessions")
        assert sum(r["role"] == "compiled_caller" for r in tls) == 32 and all(r["tls"] for r in tls)
        if label in ("thin", "trace"):
            info = member(raw, "large-diagnostic")
            assert info["mode"] == label
            observed["physical"] = physical_check(member(raw, "large-physical"), payloads, lane)
            observed["diagnostic"] = info
        if label == "thin":
            assert observed["fallback_markers"] == observed["sql"]["set"]["calls"]
            assert observed["empty_markers"] == observed["set_lock_timeouts"] == 0
            assert info["set_before"] != info["set_after"]
        if label == "trace":
            log = raw.extractfile("adapter/postgres.log").read().decode()
            commit = result["first_commit_us"]
            out = analyze(log, commit, commit + round(observed["last_grant_ms"] * 1000),
                          result["last_settlement_us"])
            assert len(out["sequences"]) == observed["fallback_markers"]
            observed["trace"] = out
    del observed["payload_sha256"]
    return observed


def replay(path, cfg, block, lane, payloads):
    assert read(path / "order.json") == block
    if (path / "SHA256SUMS").is_file():
        verify_seal(path)
    arms = {f"{phase}-{label}": score(folder, phase, label, block, cfg, lane, payloads)
            for phase, label, folder in matrix.arms(path, cfg, block)}
    changes = {}
    for metric in ("last_grant_ms", "drain_ms", "submit_ms"):
        a,b,thin,trace = [arms[f"measured-{label}"][metric] for label in ("stock-a","stock-b","thin","trace")]
        changes[metric] = dict(duplicate_spread_ms=abs(a-b), thin_gain_over_faster_stock_ms=min(a,b)-thin,
                               trace_delta_from_stock_midpoint_ms=trace-(a+b)/2)
    return dict(arms=arms, changes=changes)


def decision(groups, baseline):
    if len(groups) != 2:
        return dict(advance=False, reason="need both selected reverse-order diagnostic blocks")
    changes = [group[1]["changes"] for group in groups.values()]
    submits = [b["submit_ms"]["thin_gain_over_faster_stock_ms"] for b in changes]
    metrics = {}
    for metric in ("last_grant_ms", "drain_ms"):
        spread = max(block[metric]["duplicate_spread_ms"] for block in changes)
        threshold = max(baseline["median_gap_floor_ms"], baseline["stock_drain_range_ms"], 2*spread)
        gains = [block[metric]["thin_gain_over_faster_stock_ms"] for block in changes]
        metrics[metric] = dict(threshold_ms=threshold, gains_over_faster_stock_ms=gains,
                               passed=all(gain >= threshold for gain in gains))
    advance = all(m["passed"] for m in metrics.values()) and median(submits) >= 0
    return dict(advance=advance, metrics=metrics, submit_gains_over_faster_stock_ms=submits)


def timing(lane, cfg, groups):
    active = grant(lane)
    out = output(lane)
    out.mkdir(parents=True, exist_ok=True)
    id = request_id(lane)
    series = out / "series.json"
    frozen = dict(config=cfg, plan=BLOCKS,
                  drivers={p.name: digest(p) for p in
                           (Path(matrix.__file__), SUITE/"run.py", SUITE/"verify.py", SUITE/"worker.py")},
                  remote={label: f"{id}-{label}" for label in ORDER})
    if not series.exists():
        run.write_json(series, frozen)
    assert read(series) == frozen
    run.write_json(out / f"grant-{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%f')}.json", active)
    stage = run.stage
    dispatch_block = matrix.dispatch_block

    def stage_files(path, remote, log):
        if path.name == "inventory.json":
            grant(lane)
            if remote.name.endswith(("-thin", "-trace")):
                for name in FILES:
                    stage(HERE/name, remote, log)
        stage(path, remote, log)

    def assigned(*args):
        grant(lane)
        ok = dispatch_block(*args)
        directory, _, block = args
        for attempt in matrix.attempt_paths(directory, block):
            if not (attempt / "reconciled.json").is_file():
                continue
            seal = attempt / "SHA256SUMS"
            if seal.is_file():
                verify_seal(attempt)
            else:
                paths = sorted(p for p in attempt.rglob("*") if p.is_file() and p != seal)
                with seal.open("x") as stream:
                    stream.writelines(f"{digest(p)}  {p.relative_to(attempt)}\n" for p in paths)
        return ok

    run.stage = stage_files
    matrix.dispatch_block = assigned
    return matrix.dispatch(out, cfg, groups)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("action", choices=("check", "timing", "replay"))
    cli.add_argument("lane", choices=STORAGES)
    cli.add_argument("--groups", type=int, nargs="+", choices=(1,2), default=[1,2])
    args = cli.parse_args()
    lane = args.lane
    cfg, packet, payloads = inputs(lane)
    matrix.plan = lambda *_: iter(BLOCKS)
    matrix.replay = lambda path, cfg, block: replay(path, cfg, block, lane, payloads)
    if args.action == "check":
        subprocess.run(["bash", "-n", HERE/"launch.sh"], check=True)
        for p in HERE.glob("*.py"):
            compile(p.read_bytes(), str(p), "exec")
        assert audit()["large_baseline"][lane] == packet["baseline"]
        print(f"{packet['id']}: five stock blocks, two reverse diagnostic orders, exact producer and released ELF")
        return 0
    if args.action == "timing":
        return timing(lane, cfg, args.groups)

    failures = {}
    groups = {b["group"]: selected for b in BLOCKS
              if (selected := matrix.select_attempt(output(lane), cfg, b, failures))}
    # select_attempt yields (Path, result). Render paths without altering archives.
    print(json.dumps(dict(groups=groups, failed=failures, gate=decision(groups, packet["baseline"])),
                     sort_keys=True, indent=2, default=str, allow_nan=False))
    return int(len(groups) != 2)


if __name__ == "__main__":
    sys.exit(main())
