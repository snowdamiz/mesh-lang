#!/usr/bin/env python3
"""Freeze the all-profile contracts and bind them to one controller-built ELF."""

import argparse
from collections import Counter
from copy import deepcopy
import hashlib
from importlib import import_module
import json
from pathlib import Path
import re
from statistics import median
import sys
import tarfile

HERE = Path(__file__).resolve().parent
PACKET = HERE.parent
RESULTS = PACKET.parent
SUITE = RESULTS.parent
ROOT = SUITE.parents[1]
FROZEN = RESULTS / "2026-11-24-bulk-d2-short-peers/short.json"
DENSE = RESULTS / "2026-11-23-dense-matched-topology"
TUNED = RESULTS / "2026-11-23-payload-storage-symmetric/series.json"
PEERS = ("river", "oban", "apalis")
MODES = ("product-bulk", "product-group", "product-d2")
NATIVE = Path("/var/lib/durable-jobs-bench/native")
ARM_FILES = ("native-run", "product_arm.py", "backend.py", "identity.py")
sys.path.insert(0, str(SUITE))
import run
import verify

PROFILES = verify.PROFILES
PLANS = {
    "short": ("short", "stock", "default"),
    "dense-stock": ("noop", "stock", "default"),
    "dense-mixed": ("noop", "two-runner", "default"),
    "dense-equal": ("noop", "two-runner", "default"),
    "sparse": ("sparse", "stock", "default"),
    "slow": ("slow", "stock", "default"),
    "large": ("large", "stock", "default"),
    "large-external": ("large", "stock", "external-4m"),
}
APP_SESSIONS = {"short": 8, "noop": 32, "sparse": 1, "slow": 8, "large": 32}
METRICS = ("submit_jobs_per_s", "commit_jobs_per_s", "submit_to_settlement_p99_ms",
           "commit_to_settlement_p99_ms", "peak_executing", "peak_connections")
SHORT_ORDERS = (
    ("product-bulk", "river", "product-group", "oban", "product-d2", "apalis"),
    ("product-group", "apalis", "river", "product-d2", "product-bulk", "oban"),
    ("river", "product-d2", "apalis", "product-bulk", "oban", "product-group"),
)
PEER_ORDERS = (
    ("product", "river", "oban", "apalis"),
    ("oban", "product", "apalis", "river"),
    ("apalis", "oban", "river", "product"),
)


def read(path):
    return json.loads(path.read_text())


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def plan(name, smaller=False):
    profile, topology, storage = PLANS[name]
    stock_runners, slots, cap, input_sha, warm = PROFILES[profile]
    product_runners = 2 if topology == "two-runner" else stock_runners
    peer_runners = 2 if name == "dense-equal" else stock_runners
    orders = SHORT_ORDERS if name == "short" else PEER_ORDERS
    first, second, third = orders
    orders = (first, first[::-1], second, second[::-1], third)
    if smaller:
        assert name == "short"
        orders = [tuple(lane for lane in order if lane != "product-d2") for order in orders]
    phases = ["warmup", "measured"] if warm else ["measured"]
    return dict(profile=profile, topology=topology, storage=storage, input_sha256=input_sha,
                product_runners=product_runners, peer_runners=peer_runners, slots=slots,
                cap=cap, app_sessions=APP_SESSIONS[profile],
                blocks=[dict(group=i, profile=profile, phases=phases, order=list(order))
                        for i, order in enumerate(orders, 1)])


def peers(name):
    stock = read(FROZEN)
    if name == "dense-equal":
        cfg = read(DENSE / "equal.json")
        assert all(cfg[k] == stock[k] for k in ("host", "cpus", "clock_ticks"))
        sys.path.insert(0, str(DENSE))
        try:
            equal = import_module("equal")
            equal.check_config(cfg)
            lanes = {}
            for label in PEERS:
                row = cfg["lanes"][f"{label}-2x32"]
                engine = row["engine"]
                spec, extra = equal.peer(cfg["inventory"]["engines"][engine], engine, cfg["river_equal"])
                assert not extra and row["runners"] == 2
                inventory = deepcopy(cfg["inventory"])
                inventory["engines"][engine] = spec
                lanes[label] = dict(engine=engine, inventory=inventory)
            return lanes
        finally:
            sys.path.pop(0)
    if name == "large-external":
        cfg = read(TUNED)["config"]
        assert all(cfg[k] == stock[k] for k in ("host", "cpus", "clock_ticks"))
        return {label: cfg["lanes"][f"{label}-external"] for label in PEERS}
    return {label: stock["lanes"][label] for label in PEERS}


def inventory(name, attempt, manifest_path, selection, smaller=False):
    assert re.fullmatch(r"trusted-jobs-product-[0-9a-f]{12}", attempt)
    build = read(manifest_path)
    assert (build["system"], build["architecture"]) == ("Linux", "x86_64")
    assert build["compiler_sha256"] == "e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19"
    assert build["execution_contract"] == "ordinary-trusted-shared-process-v2"
    assert attempt.endswith(build["source_sha256"][:12])
    lanes = peers(name)
    frozen = read(FROZEN)
    cfg = {key: frozen[key] for key in ("host", "cpus", "clock_ticks", "contract")}
    recipe = plan(name, smaller)
    cfg.update(description=f"Trusted Jobs product {name}. Each arm uses a fresh PostgreSQL cluster.",
               profiles=[recipe["profile"]], lanes=lanes)
    remote = NATIVE / f"source-{attempt}/source" / PACKET.relative_to(ROOT) / "replay"
    artifact = NATIVE / f"builds/{attempt}"
    manifest_sha = sha(manifest_path)
    elf = build["files"]["mesh"]
    pins = {str(remote / name): sha(HERE / name) for name in ARM_FILES}
    pins.update({str(artifact / name): checksum for name, checksum in build["files"].items()})
    pins[str(artifact / "mesh.json")] = manifest_sha
    choices = [(label, label) for label in (MODES[:2] if smaller else MODES)] if name == "short" else [("product", selection)]
    for label, mode in choices:
        entry = deepcopy(frozen["lanes"]["bulk"])
        spec = entry["inventory"]["engines"]["mesh"]
        # Keep the qualified harness input pins. Drop the old V2 runtime files.
        harness = {path: value for path, value in spec["verified_files"].items()
                   if "/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2/" in path}
        spec.update(argv=["/usr/bin/env", f"PRODUCT_ARTIFACT={artifact}",
                          f"PRODUCT_COMPOSITION={mode}", f"PRODUCT_TOPOLOGY={recipe['topology']}",
                          f"PRODUCT_STORAGE={recipe['storage']}", f"PRODUCT_ELF_SHA256={elf}",
                          f"PRODUCT_MANIFEST_SHA256={manifest_sha}", str(remote / "native-run"), "mesh"],
                    artifact_sha256=manifest_sha, version=build["version"], verified_files=harness | pins)
        entry["queue_policy"] = "set_based" if mode == "product-bulk" else "success_cohort"
        entry["product_mode"] = mode
        lanes[label] = entry
    for lane in lanes.values():
        run.check_inventory(lane["inventory"])
    assert all(set(block["order"]) == set(lanes) for block in recipe["blocks"])
    return {"configuration": cfg, "recipe": recipe, "artifact": build, "manifest_sha256": manifest_sha}


def compare(qualified, block_root, group):
    cfg = qualified["configuration"]
    recipe = qualified["recipe"]
    block = recipe["blocks"][group - 1]
    assert read(block_root / "order.json") == block
    result = {}
    for phase in block["phases"]:
        for label in block["order"]:
            lane = cfg["lanes"][label]
            engine = lane["engine"]
            folder = block_root / f"{phase}-{label}" / f"{phase}-{engine}"
            stock = PROFILES[recipe["profile"]]
            runners = recipe["product_runners" if engine == "mesh" else "peer_runners"]
            PROFILES[recipe["profile"]] = (runners, *stock[1:])
            try:
                score = run.score_arm(folder, block, phase, engine, lane["inventory"])
            finally:
                PROFILES[recipe["profile"]] = stock
            assert [score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"]] == cfg["host"]
            archive_path = folder / "raw.tar.gz"
            if engine == "mesh":
                with tarfile.open(archive_path) as archive:
                    get = lambda name: run.file_json(archive, f"adapter/{name}.json")
                    assert get("build") == qualified["artifact"]
                    ledger = get("ledger")
                    q = get("queue-policy")
                    assert q["queue_policy"] == ledger["queue_policy"] == lane["queue_policy"]
                    assert q["admitted_queries"] > 0
                    identities = get("process-identity")
                    assert len(identities) == runners + 1
                    assert get("postgres-settings")["max_connections"] == recipe["cap"]
                    storage = get("storage-topology")
                    assert storage["topology"] == recipe["topology"] and storage["storage"] == recipe["storage"]
                    assert storage["work_mem"] == ("16MB" if recipe["storage"] == "external-16m" else "4MB")
                    assert storage["args_storage"] == ("x" if recipe["storage"] == "default" else "e")
                    writes = get("trusted-application-statements")
                    for suffix in ("attempts", "effects"):
                        (statement,) = [s for s in writes
                                       if s["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + suffix)]
                        assert statement["rows"] == statement["calls"] == score["completed"]
                    probes = sum(row["calls"] for row in writes if row["query"] == "SELECT $1")
                    cohorts = Counter(row["settled_xid"] for row in ledger["jobs"])
                    assert max(cohorts.values()) <= (1 if lane["product_mode"] == "product-bulk" else 8)
                    score.update(checkout_probes=probes, terminal_xids=len(cohorts),
                                 largest_terminal_cohort=max(cohorts.values()))
                    if recipe["profile"] == "short":
                        writer = get("producer-writer")
                        assert (writer["bulk"], writer["scalar"]) == (1, 0)
                        assert (probes < 256 if lane["product_mode"] == "product-d2" else probes == 256)
            if phase == "measured":
                result[label] = dict(score, raw_sha256=sha(archive_path))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    sub.add_parser("plans")
    bind = sub.add_parser("bind")
    bind.add_argument("plan", choices=PLANS)
    bind.add_argument("attempt")
    bind.add_argument("manifest", type=Path)
    bind.add_argument("--selection", choices=MODES, default="product-bulk")
    bind.add_argument("--smaller", action="store_true")
    check = sub.add_parser("replay")
    check.add_argument("qualified", type=Path)
    check.add_argument("blocks", nargs="+", type=Path)
    args = parser.parse_args()
    if args.action == "plans":
        data = {name: {**plan(name), "peers": peers(name)} for name in PLANS}
        data["short-smaller"] = {**plan("short", True), "peers": peers("short")}
    elif args.action == "bind":
        data = inventory(args.plan, args.attempt, args.manifest, args.selection, args.smaller)
    else:
        qualified = read(args.qualified)
        assert len(args.blocks) <= 5
        groups = [compare(qualified, path, group) for group, path in enumerate(args.blocks, 1)]
        data = dict(groups=groups, medians={
            lane: {key: median(group[lane][key] for group in groups) for key in METRICS}
            for lane in groups[0]},
            paired_submit_drain_wins={
                lane: {peer: {key: sum(group[lane][key] > group[peer][key] for group in groups)
                              for key in METRICS[:2]} for peer in PEERS}
                for lane in groups[0] if lane.startswith("product")})
    print(json.dumps(data, sort_keys=True, indent=2))


if __name__ == "__main__":
    main()
