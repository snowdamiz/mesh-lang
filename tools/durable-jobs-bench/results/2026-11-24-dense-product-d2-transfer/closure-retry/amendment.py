#!/usr/bin/env python3
"""Use the qualified public adapter repair in the signed dense checkout campaign."""
import argparse
from collections import Counter
from datetime import datetime, timezone
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import transfer as dense

ID = dense.ID + "-closure-retry"
REQUEST = HERE / "request.json"
PUBLIC = dense.RESULTS / dense.PRODUCT
SHORT_ROOT = PUBLIC / "evidence/native/timing-closure-retry"
SHORT = SHORT_ROOT / "short-smaller"
ORIGINAL_BINDING = HERE.parent / "evidence/product-setup-binding"
ELF = "af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355"
MANIFEST = "979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55"
ATTEMPT = f"trusted-jobs-product-{dense.PRODUCT_SOURCE[:12]}"
SETUP = PUBLIC / "evidence/native/setup" / ATTEMPT
RETRY = SETUP.with_name(ATTEMPT + "-closure-retry")
OLD = dense.BASE / f"source-{ATTEMPT}" / "source" / dense.TARGET / "replay"
OVERLAY = OLD.with_name("replay-closure-retry")
NAMES = ("native-run", "product_arm.py", "identity.py", "backend.py")
OUTPUT = HERE.parent / "evidence/native-closure-retry"
run, matrix, require = dense.run, dense.matrix, dense.require
original_stages = dense.stages


def request():
    subprocess.run(["openssl", "pkeyutl", "-verify", "-pubin", "-inkey", HERE / "request-public.pem",
                    "-rawin", "-in", REQUEST, "-sigfile", HERE / "request.sig"],
                   check=True, stdout=subprocess.DEVNULL)
    document = run.read_json(REQUEST)
    require(document["id"] == ID and document["original_request_sha256"] == run.digest(dense.REQUEST)
            and document["build_free"] is True and document["product_executable"] == ELF,
            "unexpected amended request")
    for relative, expected in document["local_sha256"].items():
        require(run.digest(dense.ROOT / relative) == expected, f"amendment pin changed: {relative}")
    return document


def frozen_adapter():
    with tarfile.open(PUBLIC / "source.tar.gz", "r:gz") as raw:
        get = dense.archive_files(raw, f"source/{dense.TARGET}/replay/")
        return {name: get(name) for name in (*NAMES, "intake.py")}


def bind():
    public = dense.setup_binding(ORIGINAL_BINDING / "controller-release.json", SETUP / "raw.tar.gz")
    public["request_sha256"] = run.digest(dense.REQUEST)
    require(public == run.read_json(ORIGINAL_BINDING / "binding.json")
            and public["manifest_sha256"] == MANIFEST and public["build"]["files"]["mesh"] == ELF,
            "amendment must retain the verified original SETUP binding")
    retry_release = run.read_json(RETRY / "controller-release.json")
    require(retry_release["id"] == dense.PRODUCT + "-closure-retry" and retry_release["exit_status"] == 0
            and run.read_json(RETRY / "status.json")["exit_code"] == 0, "closure retry not released")
    frozen = frozen_adapter()
    overlay_hashes = {str(OVERLAY / name): dense.digest(
        (PUBLIC / "closure-retry/backend.py").read_bytes() if name == "backend.py" else frozen[name])
        for name in NAMES}
    with tarfile.open(RETRY / "raw.tar.gz", "r:gz") as raw:
        get = dense.archive_files(raw)
        require(get("SETUP-PASS") == b"" and dense.digest(get("mesh.json")) == MANIFEST,
                "closure retry did not qualify the product ELF")
        files = dict(pair.split()[::-1] for pair in get("adapter.sha256").decode().splitlines())
        require(files == overlay_hashes, "closure retry installed unexpected files")
        arms = {}
        for mode, policy, cohort_cap in (("product-bulk", "set_based", 1),
                                         ("product-group", "success_cohort", 8)):
            def read(name):
                return json.loads(get(f"{mode}/{name}.json"))

            ledger = read("ledger")
            row = read("row")
            require(ledger["phase"] == row["phase"] == "diagnostic" and not row["local_smoke"]
                    and read("build") == public["build"], "wrong native diagnostic")
            jobs = ledger["jobs"]
            app = {job[k] for job in jobs for k in ("attempt_xid", "effect_xid")}
            terminal = Counter(job["settled_xid"] for job in jobs)
            origin, = ledger["transactions"]
            queue = read("queue-policy")
            writer = read("producer-writer")
            require(len(jobs) == 128 and all(job["state"] == "completed" for job in jobs)
                    and len(app) == 256 and not (app & terminal.keys())
                    and str(int(origin["xid"]) % 2**32) not in app | terminal.keys()
                    and queue["queue_policy"] == policy and queue["admitted_queries"] > 0
                    and max(terminal.values()) <= cohort_cap
                    and (writer["bulk"], writer["scalar"]) == (1, 0), "closure retry lifecycle proof failed")
            arms[mode] = dict(jobs=len(jobs), application_xids=len(app), terminal_xids=len(terminal))
    proof = dict(files=overlay_hashes, executable_sha256=ELF, manifest_sha256=MANIFEST,
                 raw_sha256=run.digest(RETRY / "raw.tar.gz"), arms=arms)
    require(run.read_json(RETRY / "qualification.json") == proof, "closure qualification differs from raw replay")
    return dict(original=public, closure_retry=proof, request_sha256=run.digest(REQUEST))


def stages(config, label, remote):
    files, inventory = original_stages(config, label, remote)
    if label in dense.PEERS:
        return files, inventory
    spec = inventory["engines"]["mesh"]
    for name in NAMES:
        before, after = str(OLD / name), str(OVERLAY / name)
        previous = spec["verified_files"].pop(before)
        pinned = config["closure_retry"]["files"][after]
        require((previous != pinned) == (name == "backend.py") and after not in spec["verified_files"],
                "amendment changed an unrelated adapter file")
        spec["verified_files"][after] = pinned
    old_launcher, new_launcher = str(OLD / "native-run"), str(OVERLAY / "native-run")
    require(spec["argv"].count(old_launcher) == 1, "original launcher changed")
    spec["argv"] = [new_launcher if arg == old_launcher else arg for arg in spec["argv"]]
    run.check_inventory(inventory)
    return files, inventory


def verify_staging(config):
    inventories = {}
    for label in config["lanes"]:
        remote = run.REMOTE / "offline-closure-retry"
        old_files, original = original_stages(config, label, remote)
        files, inventory = stages(config, label, remote)
        require(old_files == files, "amendment stages extra files")
        if label in dense.PEERS:
            require(inventory == original, f"peer {label} changed")
            continue
        spec = inventory["engines"]["mesh"]
        old_spec = original["engines"]["mesh"]
        require({k: v for k, v in spec.items() if k not in ("argv", "verified_files")} ==
                {k: v for k, v in old_spec.items() if k not in ("argv", "verified_files")},
                "unexpected Mesh invocation or artifact change")
        before = {str(OLD / name) for name in NAMES}
        after = {str(OVERLAY / name) for name in NAMES}
        require({k: v for k, v in spec["verified_files"].items() if k not in after} ==
                {k: v for k, v in old_spec["verified_files"].items() if k not in before}
                and spec["argv"] == [str(OVERLAY / "native-run") if arg == str(OLD / "native-run")
                                     else arg for arg in old_spec["argv"]],
                "closure retry changed any other source, binary, topology, or mode")
        inventories[label] = spec
    baseline = inventories["checked-a"]
    require(baseline == inventories["checked-b"], "duplicate checked controls differ")
    hot = dict(baseline, argv=["PRODUCT_COMPOSITION=product-d2" if arg == "PRODUCT_COMPOSITION=product-group"
                               else arg for arg in baseline["argv"]])
    require(hot == inventories["hot"], "hot lane differs beyond its checkout option")


def short_replay():
    path = PUBLIC / "replay/intake.py"
    require(path.read_bytes() == frozen_adapter()["intake.py"], "public scorer differs from the sealed source")
    spec = importlib.util.spec_from_file_location("dense_public_short_scorer", path)
    scorer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(scorer)
    bound = run.read_json(SHORT_ROOT / "short-smaller.inventory.json")
    config, recipe = bound["configuration"], bound["recipe"]
    require(bound["artifact"]["files"]["mesh"] == ELF and bound["manifest_sha256"] == MANIFEST
            and recipe["product_runners"] == 1, "short campaign bound another ELF or topology")
    series = run.read_json(SHORT / "controller-evidence/series.json")
    require(series["config"] == config and series["plan"] == recipe["blocks"], "short schedule or config changed")
    selected = run.read_json(SHORT / "controller-evidence/summary.json")
    # Keep replay paths relative to the portable evidence rather than the original workstation.
    paths = sorted((SHORT / "blocks").iterdir())
    require(len(paths) == 5, "expected five retained repaired blocks")
    groups = {}
    for group, folder in enumerate(paths, 1):
        scores = scorer.compare(bound, folder, group)
        require(selected["groups"][group - 1] == scores
                and run.read_json(folder / "reconciled.json") == {"scores": scores}
                and all(row["completed"] == 128 and row["failed"] == 0 for row in scores.values()),
                f"short group {group} differs from its checkpoint")
        groups[str(group)] = dict(attempt=folder.name, raw={lane: row["raw_sha256"] for lane, row in scores.items()})
    require(len(groups) == 5 and len({s for row in groups.values() for s in row["raw"].values()}) == 25,
            "short raw archive reused")
    for window in ("1-2", "3-5"):
        name = f"{dense.PRODUCT}-short-smaller-closure-retry-groups-{window}.released.json"
        release = run.read_json(SHORT / "controller-evidence" / name)
        require(release["id"] == f"{dense.PRODUCT}-short-smaller-closure-retry" and release["exit_status"] == 0,
                "short grant did not release successfully")
    require(len(selected["groups"]) == 5, "published short snapshot is incomplete")
    return groups


def grant(_=None):
    mode, identifier, start, end = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+) (\S+)(?: (\S+))?",
                                               (dense.QUEUE / "STATUS.md").read_text(), re.MULTILINE)[-1]
    require((mode, identifier) == ("TIMING", ID)
            and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "dedicated controller TIMING grant required")
    require(REQUEST.read_bytes() == (dense.QUEUE / f"{ID}.json").read_bytes(), "amended inbox bytes changed")
    require(dense.REQUEST.read_bytes() == (dense.QUEUE / f"{dense.ID}.json").read_bytes(),
            "original signed booking changed")
    return dict(start=start, end=end, request_sha256=run.digest(REQUEST))


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("action", choices=("check", "bind", "timing", "replay"))
    cli.add_argument("groups", nargs="*", type=int, choices=range(1, 7))
    args = cli.parse_args()
    dense.request()
    document = request()
    dense.check_original()
    binding = bind()
    blocks = dense.schedule(run.read_json(dense.PLAN))
    require(blocks == document["group_plan"], "amendment changes the six fixed orders")
    groups = short_replay()
    require(groups == run.read_json(HERE / "short-raw-replay.json"), "short raw replay changed")
    config = dense.configurations(binding["original"])
    config["closure_retry"] = binding["closure_retry"]
    dense.stages = stages
    matrix.stages, matrix.grant = stages, grant
    matrix.replay_block, matrix.summarize = dense.replay_block, dense.summarize
    matrix.SERIES[ID] = ID
    pins = dict(original_request=run.digest(dense.REQUEST), amendment=run.digest(REQUEST),
                **document["local_sha256"])
    if args.action == "bind":
        print(json.dumps(binding, indent=2, sort_keys=True))
        return 0
    if args.action == "check":
        verify_staging(config)
        print("Original SETUP and closure retry verified on the same ELF; five short groups and 25 raw arms replayed.")
        return 0
    if args.action == "replay":
        result = dense.summarize(OUTPUT, config, blocks, pins)
        print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
        return bool(result["missing"])

    grant()
    require(args.groups in ([1, 2], [3, 4, 5, 6]), "use the booked pilot or continuation")
    if args.groups[0] == 3:
        require(not {1, 2} & set(dense.summarize(OUTPUT, config, blocks, pins)["missing"]),
                "both pilot rotations must reconcile before continuation")
    original = run.dispatch_arm

    def dispatch_checked(path, remote, block, phase, engine):
        original(path, remote, block, phase, engine)
        folder = path / f"{phase}-{engine}"
        require(run.read_json(folder / "dispatch.json").get("exit_code") == 0, "retain failed native arm")
        dense.score_arm(folder, phase, path.name.removeprefix(phase + "-"), config, remote)

    run.dispatch_arm = dispatch_checked
    OUTPUT.parent.mkdir(exist_ok=True, parents=True)
    matrix.dispatch(OUTPUT, config, ID, blocks, pins, args.groups)
    return bool(set(args.groups) & set(dense.summarize(OUTPUT, config, blocks, pins)["missing"]))


if __name__ == "__main__":
    sys.exit(main())
