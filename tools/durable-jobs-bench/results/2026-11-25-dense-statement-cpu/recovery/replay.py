"""Recover the original CPU archives offline with one OID schema correction."""
import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path
import re
import sys
import tarfile
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
ORIGINAL = HERE.parent
sys.path.insert(0, str(ORIGINAL))

from measure import attribute, require

OID_MAX = (1 << 32) - 1


def oid(value, dictionary):
    if dictionary:
        require(type(value) is str and re.fullmatch(r"[1-9][0-9]{0,9}", value),
                "dictionary OID must be canonical unsigned decimal text")
        value = int(value)
    require(type(value) is int and 0 < value <= OID_MAX, "hook OID type or OID range differs")
    return value


def query_id(value):
    require(type(value) is str and re.fullmatch(r"0|-?[1-9][0-9]{0,18}", value)
            and -(1 << 63) <= int(value) < (1 << 63), "query ID must be canonical signed int64 text")
    return value


def key(row, dictionary):
    return oid(row["db"], dictionary), oid(row["role"], dictionary), query_id(row["query"])


def corrected_catalog(catalog):
    rows = []
    seen = set()
    for row in catalog["rows"]:
        db, role, query = identity = key(row, dictionary=True)
        require(identity not in seen, "duplicate normalized dictionary key")
        seen.add(identity)
        rows.append(dict(row, db=db, role=role, query=query))
    return dict(catalog, rows=rows)


def corrected_attribute(content, catalog, enabled, engine, roster):
    for row in map(json.loads, content.splitlines()):
        if "slot" in row:
            key(row, dictionary=False)
    return attribute(content, corrected_catalog(catalog), enabled, engine, roster)


def old_controller():
    spec = importlib.util.spec_from_file_location("sealed_statement_cpu", ORIGINAL / "controller.py")
    controller = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(controller)
    return controller


FAMILIES = {"enqueue", "grant", "settle", "application-write", "observer", "transaction"}


def calls(catalog, engine):
    counts = Counter()
    for row in corrected_catalog(catalog)["rows"]:
        if row["database"] == engine and row["family"] in FAMILIES:
            require(type(row["calls"]) is int and row["calls"] > 0, "invalid dictionary call count")
            counts[row["family"]] += row["calls"]
    require(FAMILIES <= counts.keys() and counts["application-write"] == 2048
            and counts["enqueue"] == 8 and (counts["settle"] == 1024 if engine == "mesh" else counts["settle"] > 0),
            f"frozen workloads differ: {engine} {dict(counts)}")
    return dict(sorted(counts.items()))


def workload_rows(pg, engine, dictionary_counts):
    authorities = {
        "mesh": {"grant": "parity_subject", "settle": "parity_subject",
                 "enqueue": "parity_producer", "application-write": "compiled_caller"},
        "river": dict.fromkeys(("grant", "settle", "enqueue", "application-write"), "parity_subject"),
    }[engine]
    rows = []
    for family, authority in authorities.items():
        matched = [entry for entry in pg["top_level"] if entry["dictionary"]
                   and entry["dictionary"]["family"] == family]
        require(len(matched) == (2 if family == "application-write" else 1)
                and all(r["login"] == r["dictionary"]["effective_role"] == authority and r["database"] == engine
                        and r["calls"] == [r["dictionary"]["calls"]] * 4 + [0] and not r["errors"] for r in matched)
                and sum(r["calls"][1] for r in matched) == dictionary_counts[family],
                f"hook counts/role disagree with workload: {engine} {family}")
        rows.extend(dict(family=family, login=row["login"], query=row["query"],
                         sql_sha256=row["dictionary"]["sql_sha256"], calls=row["calls"][1],
                         full_session_hook_cpu_ns=row["cpu_ns"]) for row in matched)
    return rows


def archive_counts(controller, result):
    directory = controller.OUT / "timing"
    manifest = controller.run.read_json(directory / "series.json")
    arms = []
    for block in controller.BLOCKS:
        group = str(block["group"])
        prefix = controller.run.block_prefix(block)
        attempt, = (directory / "blocks").glob(f"{prefix}.*")
        for phase, lane, _, folder in controller.matrix.paths(attempt, manifest["config"], block):
            engine = lane.split("-", 1)[0]
            dispatch = controller.run.read_json(folder / "dispatch.json")
            path = folder / "raw.tar.gz"
            require(dispatch.get("exit_code") == 0 and "interrupted" not in dispatch, "arm did not exit successfully")
            with tarfile.open(path) as raw:
                catalog = controller.run.file_json(raw, "adapter/dense-statement-keys.json")
                counts = calls(catalog, engine)
                if engine == "mesh":
                    policy = controller.run.file_json(raw, "adapter/queue-policy.json")
                    queue_rows = policy["queue_statements"]
                    for family, prefix in (("grant", "FROM mesh_jobs.claim_batch_set_v1("),
                                           ("settle", "SELECT coalesce(mesh_jobs.settle_state(")):
                        total = sum(row["calls"] for row in queue_rows if prefix in row["query"])
                        require(total == counts[family], f"Mesh queue witness disagrees with {family} count")
            arms.append(dict(group=group, phase=phase, lane=lane, raw_sha256=controller.run.digest(path),
                             calls=counts))
            if phase == "measured" and lane.endswith("-on"):
                arm = result["measured"][group][lane]
                arm["workload_hook_rows"] = workload_rows(arm["postgres"], engine, counts)
    require(len(arms) == 24 and len({arm["raw_sha256"] for arm in arms}) == 24, "missing or repeated raw archive")
    return arms


def recover():
    controller = old_controller()
    controller.packet()
    cfg, _ = controller.historical()
    controller.release("setup")
    files = controller.setup_files()
    original_path = controller.OUT / "timing/controller-release.json"
    original_release = controller.run.read_json(original_path)
    require(original_release["exit_status"] == 1 and original_release["released_at"]
            and original_release["id"] == f"{controller.ID}-timing"
            and original_release["request_sha256"] == controller.run.digest(controller.PACKET),
            "original diagnostic failure is missing")
    log_path = original_path.with_name("controller.log")
    log = log_path.read_text()
    failures = re.findall(r"^(0[12])-noop\.\d+ failed qualification: (.+)$", log, re.M)
    require(failures == [(str(i).zfill(2), f"expected workload statements missing: {{'unmapped': {count}}}")
                         for i, count in ((1, 3761), (2, 2309))], "original scorer failures differ")
    # Preserve all original checks, dispatch order and perturbation math. The
    # wrapper only validates identity fields and converts the two dictionary OIDs.
    with patch.object(controller, "attribute", corrected_attribute):
        result = controller.replay_result(cfg, files)
    result["archives"] = archive_counts(controller, result)
    result["provenance"] = dict(request_sha256=original_release["request_sha256"],
                                timing_log_sha256=controller.run.digest(log_path),
                                original_release_sha256=controller.run.digest(original_path),
                                original_setup_sha256=controller.run.digest(controller.OUT / "setup/receipt.json"),
                                original_failures=dict(failures), original_release=original_release)
    return result


def compact(result):
    for lanes in result["measured"].values():
        for arm in lanes.values():
            arm["postgres"].pop("top_level")
            arm["postgres"].pop("logins")
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--full", action="store_true", help="include every top-level key and login")
    args = parser.parse_args()
    result = recover()
    print(json.dumps(result if args.full else compact(result), sort_keys=True, indent=2, allow_nan=False))
