#!/usr/bin/env python3
"""Freeze and dispatch a bounded slow-storage crossover through the Linode queue."""
import argparse
from collections import Counter
from copy import deepcopy
from datetime import datetime, timedelta, timezone
import fcntl
import json
from pathlib import Path
from statistics import median
import re
import sys
import tarfile
from uuid import uuid4

import native_overlay
import storage

HERE = Path(__file__).resolve().parent
PACKET = HERE.parent
SUITE = PACKET.parents[1]
ROOT = SUITE.parents[1]
ID = "ordinary-slow-storage-transfer"
REQUESTS = ROOT / "target/linode-jobs-stepchange/requests"
FROZEN = PACKET / "frozen"
OUT = PACKET / "evidence/native"
NATIVE_BASE = Path("/var/lib/durable-jobs-bench/native")
FILES = ("native-run", "native_overlay.py", "slow-river.sql", "storage.py")
STOCK = {"mesh": "bulk", "oban": "oban", "go": "river", "rust": "apalis"}
HOST_IDLE = f"""
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<{NATIVE_BASE}/native.lock
flock -n 9
"""
INSTALL = HOST_IDLE + """
umask 022
destination=DESTINATION
work=$(mktemp -d /var/lib/durable-jobs-bench/native/slow-storage.partial.XXXXXX)
trap 'rm -rf -- "$work"' EXIT
cat >"$work/package.tar"
printf '%s  %s\n' PACKAGE_SHA "$work/package.tar" | sha256sum -c
mkdir "$work/overlay"
tar --no-same-owner -xf "$work/package.tar" -C "$work/overlay"
if [[ -e "$destination" ]]; then
    diff -rq -- "$work/overlay" "$destination"
else
    mv -T -- "$work/overlay" "$destination"
fi
printf '%s\n' "$destination"
"""

sys.path.insert(0, str(SUITE))
import run as matrix


def file_hash(path):
    return matrix.digest(path)


def order(lanes):
    # Williams crossover order. The starting positions alternate the
    # policies and scatter engine pairs over the five bounded rotations.
    offsets = (0, 1, -1, 2, -2, 3, -3, 4)
    return [[lanes[(start + offset) % len(lanes)] for offset in offsets] for start in (0, 5, 2, 7, 4)]


def pinned_config(source, remote):
    lanes = {}
    for engine, name in STOCK.items():
        original = source["lanes"][name]
        for policy in ("pglz", "lz4"):
            lane = deepcopy(original)
            spec = lane["inventory"]["engines"][engine]
            if engine in ("mesh", "oban"):
                _, *parameters = spec["argv"]
                *env, launcher, command = parameters
                env = [arg for arg in env if not arg.startswith("MESH_JOBS_SET_BASED_ENQUEUE=")]
                adapter = Path(launcher).with_name("arm.py")
                if command != engine:
                    raise ValueError(f"{engine}: invalid stock wrapper argument")
                spec["argv"] = ["/usr/bin/env", *env, str(remote / "native-run"), engine, policy, str(adapter)]
            else:
                launcher, *args = spec["argv"]
                adapter = Path(launcher).with_suffix(".py")
                spec["argv"] = ["/usr/bin/python3", "-B", str(remote / "native_overlay.py"),
                                engine, policy, str(adapter), *args]
            if spec["verified_files"].get(str(adapter)) != native_overlay.NATIVE_PINS[engine]:
                raise ValueError(f"{engine}: exact stock adapter is not pinned")
            spec["verified_files"].update({str(remote / name): file_hash(HERE / name) for name in FILES})
            spec["evidence_globs"] = sorted(set(spec["evidence_globs"]) | {"payload-storage.json"})
            matrix.check_inventory(lane["inventory"])
            lanes[f"{STOCK[engine]}-{policy}"] = dict(lane, policy=policy)
    labels = ("bulk-pglz", "river-lz4", "oban-pglz", "apalis-lz4",
              "bulk-lz4", "river-pglz", "oban-lz4", "apalis-pglz")
    postgres = matrix.read_json(FROZEN / "native-postgres-capability.json")
    apalis_pg = source["lanes"]["apalis"]["inventory"]["engines"]["rust"]["verified_files"]
    if apalis_pg.get(postgres["postgres_path"]) != postgres["postgres_sha256"]:
        raise ValueError("PostgreSQL capability receipt is for a different native binary")
    return {**source, "lanes": lanes, "orders": order(labels), "remote": str(remote), "postgres": postgres,
            "description": "Same compiled slow input, eight arms crossing cluster PGLZ and LZ4 on all four engines."}


def freeze():
    out = FROZEN / "native-overlay.tar"
    with tarfile.open(out, "w", format=tarfile.USTAR_FORMAT) as archive:
        for name in FILES:
            entry = tarfile.TarInfo(name)
            entry.mode = 0o755 if name == "native-run" else 0o644
            entry.size = (HERE / name).stat().st_size
            with (HERE / name).open("rb") as source:
                archive.addfile(entry, source)
    digest = file_hash(out)
    remote = NATIVE_BASE / f"slow-storage-{digest[:16]}"
    config = pinned_config(matrix.read_json(FROZEN / "native-source-inventory.json"), remote)
    storage.write(FROZEN / "native-config.json", config)
    (FROZEN / "native-install.sh").write_text(
        INSTALL.replace("DESTINATION", str(remote)).replace("PACKAGE_SHA", digest))


def check():
    config = matrix.read_json(FROZEN / "native-config.json")
    original = matrix.read_json(FROZEN / "native-source-inventory.json")
    archive = FROZEN / "native-overlay.tar"
    remote = NATIVE_BASE / f"slow-storage-{file_hash(archive)[:16]}"
    if config != pinned_config(original, remote):
        raise ValueError("frozen native inventory and local inputs diverged")
    with tarfile.open(archive) as package:
        if package.getnames() != list(FILES):
            raise ValueError("frozen overlay member set differs")
        for name in FILES:
            if package.extractfile(name).read() != (HERE / name).read_bytes():
                raise ValueError(f"frozen overlay member changed: {name}")
    request_path = PACKET / "request.json"
    if request_path.is_file():
        request = matrix.read_json(request_path)
        for relative, digest in request["local_sha256"].items():
            if file_hash(ROOT / relative) != digest:
                raise ValueError(f"changed request input: {relative}")
    return config


def verify_storage(receipt, ledger, engine, policy):
    storage.verify_snapshot(engine, receipt["persisted"], ledger["jobs"], policy)
    if receipt["engine"] != engine or len(ledger["transactions"]) != 1:
        raise ValueError("wrong engine or unexpected enqueue transaction count")
    setup, persisted = receipt["setup"], receipt["persisted"]
    transaction, = ledger["transactions"]
    if not (setup["metadata"] == persisted["metadata"] and setup["rows"] == 0
            and 0 < setup["checked_us"] < transaction["submitted_us"]
            and max(row["settled_us"] for row in ledger["jobs"]) <= receipt["verify_start_us"]
            <= receipt["verify_end_us"]):
        raise ValueError("column policy or storage snapshot crossed the timed workload")
    if engine != "rust" and setup["producer_session_policy"] != policy:
        raise ValueError("live producer session used the wrong TOAST default")
    if engine == "rust" and not setup["no_database_or_role_override"]:
        raise ValueError("Apalis cluster TOAST default has an unverified role override")
    if engine == "go" and not receipt["triggers_retired"]:
        raise ValueError("River layout/snapshot event triggers did not retire")
    answer = dict(compression_counts=dict(Counter(str(row["compression"]) for row in persisted["rows"])),
                  column_bytes=sum(row["stored_column_bytes"] for row in persisted["rows"]),
                  **{field: persisted[field] for field in ("heap_bytes", "toast_heap_bytes", "total_bytes")})
    if engine == "mesh":
        bulk = [row for row in receipt["writer"] if row["query"].startswith("SELECT ordinal, receipt")]
        scalar = [row for row in receipt["writer"] if row["query"].startswith("SELECT mesh_jobs.enqueue(")]
        if (sum(row["calls"] for row in bulk), sum(row["rows"] for row in bulk),
            sum(row["calls"] for row in scalar), sum(row["rows"] for row in scalar)) != (8, 504, 8, 8):
            raise ValueError("native Mesh no longer used the expected true bulk writer")
        answer.update(bulk_calls=8, bulk_rows=504, scalar_calls=8, scalar_rows=8,
                      producer_sql_exec_ms=sum(row["total_exec_time"] for row in bulk + scalar),
                      producer_sql_wal_bytes=sum(row["wal_bytes"] for row in bulk + scalar))
    returned = transaction.get("acknowledged_us", transaction.get("returned_us"))
    if returned is not None:
        if transaction["committed_us"] > returned:
            raise ValueError("producer receipt precedes COMMIT")
        answer["producer_ack_ms"] = (returned - transaction["submitted_us"]) / 1000
    return answer


def score(directory, lane, config, phase):
    engine = lane["engine"]
    folder = directory / f"{phase}-{engine}"
    score = matrix.score_arm(folder, {"profile": "slow"}, phase, engine, lane["inventory"])
    raw = folder / "raw.tar.gz"
    with tarfile.open(raw, "r:gz") as archive:
        receipt = matrix.file_json(archive, "adapter/payload-storage.json")
        ledger = matrix.file_json(archive, "adapter/ledger.json")
        if engine == "mesh":
            if ledger["execution_contract"] != config["contract"]:
                raise ValueError("wrong Mesh execution contract")
            statements = matrix.file_json(archive, "adapter/trusted-application-statements.json")
            for table in ("attempts", "effects"):
                writes = [line for line in statements
                          if line["query"].startswith(f"INSERT INTO public.mesh_jobs_parity_{table}")]
                if len(writes) != 1 or writes[0]["calls"] != 512 or writes[0]["rows"] != 512:
                    raise ValueError(f"missing independent Mesh application inserts in {table}")
        if engine in ("mesh", "oban"):
            masks = matrix.file_json(archive, "adapter/process-affinity.json")
            if len(masks) != 2 or any(mask != config["cpus"] for mask in masks.values()):
                raise ValueError("runner or PostgreSQL affinity changed")
        score.update(verify_storage(receipt, ledger, engine, lane["policy"]))
        points = [json.loads(line) for line in archive.extractfile("host-samples.jsonl")]
    head = max(index for index, point in enumerate(points)
               if point["epoch_us"] <= score["first_submit_us"])
    tail = next(index for index, point in enumerate(points)
                if point["epoch_us"] >= score["last_settlement_us"])
    ticks = [
        [last - first for first, last in zip(values, points[tail]["cpu"][name], strict=True)]
        for name, values in points[head]["cpu"].items()
    ]
    scale = 1000 / (config["clock_ticks"] * score["completed"])
    score.update(
        submit_to_last_settlement_ms=(score["last_settlement_us"] - score["first_submit_us"]) / 1000,
        commit_to_last_settlement_ms=(score["last_settlement_us"] - score["first_commit_us"]) / 1000,
        pinned_user_cpu_ms_per_job=scale * sum(row[0] + row[1] for row in ticks),
        pinned_kernel_cpu_ms_per_job=scale * sum(row[2] + row[5] + row[6] for row in ticks),
        observed_arm_wall_ms=(points[-1]["monotonic_ns"] - points[0]["monotonic_ns"]) / 1_000_000,
        raw_sha256=file_hash(raw),
    )
    if [score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]] != config["host"]:
        raise ValueError("host boot, kernel or PG disk changed")
    return score


def assignment(kind, seconds):
    status = (REQUESTS / "STATUS.md").read_text()
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)", status, re.M)
    if not events or events[-1][:2] != (kind, ID):
        raise ValueError(f"no current controller {kind} grant")
    _, _, start, end = events[-1]
    start_at, end_at = (datetime.fromisoformat(ts.replace("Z", "+00:00")) for ts in (start, end))
    now = datetime.now(timezone.utc)
    if not start_at <= now or now + timedelta(seconds=seconds) >= end_at:
        raise ValueError("insufficient exclusive grant time")
    if (REQUESTS / f"{ID}.json").read_bytes() != (PACKET / "request.json").read_bytes():
        raise ValueError("controller queue and portable packet request differ")
    return dict(kind=kind, from_utc=start, until_utc=end, request_sha256=file_hash(PACKET / "request.json"))


def dispatch(config, setup, groups):
    request = matrix.read_json(PACKET / "request.json")
    if request["state"] != "ready" or not config["postgres"]["lz4_available"]:
        raise ValueError("native storage transfer is held for an LZ4-capable PostgreSQL pin")
    kind = "SETUP" if setup else "TIMING"
    assignment(kind, 120)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        grant = assignment(kind, 120)
        attempt = OUT / f"{kind.lower()}-{uuid4().hex}"
        attempt.mkdir(parents=True)
        storage.write(attempt / "grant.json", grant)
        if setup:
            with (FROZEN / "native-overlay.tar").open("rb") as source, (attempt / "install.log").open("xb") as log:
                installer = (FROZEN / "native-install.sh").read_text()
                matrix.ssh(["bash", "-c", installer], stdin=source, stdout=log, stderr=log, timeout=30).check_returncode()
        else:
            if not any(OUT.glob("setup-*/PASS.json")):
                raise ValueError("missing reconciled native SETUP")
            if not groups:
                raise ValueError("controller must specify granted timing groups")
            matrix.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        remote = {}
        for label, lane in config["lanes"].items():
            remote[label] = matrix.REMOTE / f"slow-storage-{attempt.name.rsplit('-', 1)[1][:12]}-{label}"
            staging = attempt / "staging" / label
            staging.mkdir(parents=True)
            storage.write(staging / "inventory.json", lane["inventory"])
            with (staging / "transfer.log").open("xb") as log:
                matrix.stage(SUITE / "worker.py", remote[label], log)
                matrix.stage(staging / "inventory.json", remote[label], log)
        blocks = [0] if setup else groups
        for group in blocks:
            assignment(kind, 180 if setup else 240)
            labels = config["orders"][max(0, group - 1)]
            # One warmup/measurement pair per lane is charged in every timing block.
            phases = ("diagnostic",) if setup else ("warmup", "measured")
            block = attempt / f"{group:02d}-slow"
            block.mkdir()
            storage.write(block / "order.json", dict(order=labels, phases=phases, group=group))
            result = {}
            for phase in phases:
                for label in labels:
                    assignment(kind, 90)
                    lane = config["lanes"][label]
                    arm = block / f"{phase}-{label}"
                    arm.mkdir()
                    matrix.dispatch_arm(arm, remote[label], {"profile": "slow"}, phase, lane["engine"])
                    result[f"{phase}-{label}"] = score(arm, lane, config, phase)
                    storage.write(block / "checkpoint.json", result)
            storage.write(block / "reconciled.json", result)
        storage.write(attempt / "PASS.json", dict(grant=grant, groups=blocks))


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def replay(config):
    recorded = {}
    issues = {}
    for directory in sorted(OUT.glob("*/*-slow")):
        block = matrix.read_json(directory / "order.json")
        group = block["group"]
        if block["order"] != config["orders"][max(0, group - 1)]:
            raise ValueError(f"{directory}: arm order changed")
        try:
            scored = {
                f"{phase}-{label}": score(directory / f"{phase}-{label}", config["lanes"][label], config, phase)
                for phase in block["phases"] for label in block["order"]
            }
        except (OSError, ValueError, KeyError, StopIteration, tarfile.TarError) as error:
            issues[str(directory.relative_to(OUT))] = str(error)
            continue
        frozen = directory / "reconciled.json"
        if frozen.is_file() and scored != matrix.read_json(frozen):
            raise ValueError(f"{directory}: retained native score changed")
        if group:
            recorded.setdefault(group, dict(attempt=directory.parent.name, scores=scored))
    metrics = ("first_submit_to_commit_ms", "producer_ack_ms", "submit_to_last_settlement_ms",
               "commit_to_last_settlement_ms", "pinned_user_cpu_ms_per_job",
               "pinned_kernel_cpu_ms_per_job", "runner_cpu_ms_per_job",
               "commit_to_pickup_p99_ms", "effect_commit_to_settlement_p99_ms",
               "column_bytes", "toast_heap_bytes", "total_bytes", "observed_arm_wall_ms")
    measured = {}
    paired = {}
    for engine in STOCK.values():
        measured[engine] = {}
        for policy in ("pglz", "lz4"):
            label = f"measured-{engine}-{policy}"
            rows = [item["scores"][label] for item in recorded.values()]
            measured[engine][policy] = {m: distribution([r[m] for r in rows])
                                        for m in metrics if rows and m in rows[0]}
        paired[engine] = {
            m: distribution([item["scores"][f"measured-{engine}-pglz"][m]
                             - item["scores"][f"measured-{engine}-lz4"][m] for item in recorded.values()])
            for m in measured[engine]["pglz"].keys() & measured[engine]["lz4"].keys()
        }
    summary = dict(completed_groups=sorted(recorded), issues=issues, distributions=measured,
                   paired_stock_minus_lz4=paired, records=recorded)
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0 if len(recorded) == 5 else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("freeze", "check", "setup", "timing", "replay"))
    parser.add_argument("--group", type=int, choices=range(1, 6), action="append")
    args = parser.parse_args()
    if args.action == "freeze":
        return freeze()
    config = check()
    if args.action == "replay":
        return replay(config)
    if args.action != "check":
        dispatch(config, args.action == "setup", args.group)
    else:
        print(f"verified {len(config['lanes'])} inventories and overlay, {len(config['orders'])} bounded orders")


if __name__ == "__main__":
    sys.exit(main())
