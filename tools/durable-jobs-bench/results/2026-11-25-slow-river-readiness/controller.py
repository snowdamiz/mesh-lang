"""Controller-only SETUP and released-archive replay for SLOW River readiness."""

import argparse
from copy import deepcopy
from contextlib import nullcontext
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
SUITE = RESULTS.parent
ROOT = SUITE.parents[1]
STOCK = RESULTS / "2026-11-25-slow-lz4-unfused-2x32"
AMENDMENT = STOCK / "amendment"
CAP71 = RESULTS / "2026-11-24-ordinary-slow-equal-topology-cap71"
STOCK_COMMIT = "1617e257f26ec7eb3b5996406d0993918d482226"
CUSTODY_COMMIT = "c10cc82938d3754cf071c3baa8ee357247679b66"
CUSTODY = RESULTS / "2026-11-23-slow-storage-transfer/replay"
CUSTODY_FILES = ("native.py", "native_overlay.py", "storage.py", "slow-river.sql")
GO_FILES = ("db.go", "evidence.go", "main.go", "worker.go", "go.mod", "go.sum")
ID = HERE.name
OUT = HERE / "evidence"
REQUEST = HERE / "request-proof.json"
EXPERIMENT = {
    "mesh": ("two-bulk-pglz", "peer"),
    "retained-river": ("two-river-pglz", "stock"),
    "lazy-a": ("two-river-pglz", "cold"),
    "warm-river": ("two-river-pglz", "warm"),
    "lazy-b": ("two-river-pglz", "cold"),
    "oban": ("two-oban-pglz", "peer"),
    "underready-river": ("two-river-pglz", "underready"),
}


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha(data):
    return hashlib.sha256(data).hexdigest()


def build_sources(base, work):
    prior = base.read(CAP71 / "evidence/river-build.json")
    stock, candidate = (work / name for name in ("stock", "candidate"))
    stock.mkdir()
    candidate.mkdir()
    for name in GO_FILES:
        path = f"tools/durable-jobs-bench/go/{name}"
        data = subprocess.check_output(["git", "show", f"{STOCK_COMMIT}:{path}"], cwd=ROOT)
        base.require(sha(data) == prior["stock_source"][path], f"retained River source differs: {name}")
        if name == "main.go":
            before, after = (prior[key].encode() for key in ("before", "after"))
            base.require(data.count(before) == 1, "retained cap-71 profile anchor differs")
            data = data.replace(before, after)
            base.require(sha(data) == prior["patched_main_sha256"], "retained cap-71 main source differs")
        (stock / name).write_bytes(data)
        (candidate / name).write_bytes(data)
    subprocess.run(["patch", "--batch", "--fuzz=0", "-p1", "-i", HERE / "adapter.patch"],
                   cwd=candidate, check=True, stdout=subprocess.DEVNULL)
    (candidate / "readiness.go").write_bytes((HERE / "readiness.go").read_bytes())
    expected = [*GO_FILES, "readiness.go"]
    base.require(sorted(p.name for p in candidate.iterdir()) == sorted(expected),
                 "readiness patch created an unsealed file")
    info = ("\n".join(prior["build_info"]) + "\n").encode()
    (work / "expected-build-info.txt").write_bytes(info)
    paths = [work / "expected-build-info.txt", *(stock / name for name in GO_FILES),
             *(candidate / name for name in expected)]
    checks = "".join(f"{base.sha(path)}  {path.relative_to(work).as_posix()}\n" for path in paths)
    checksum_file = work / "source-sha256.txt"
    checksum_file.write_text(checks)
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as archive:
        for path in [*paths, checksum_file]:
            archive.add(path, arcname=path.relative_to(work).as_posix())
    return buf.getvalue(), checks


def validate_packet(custody):
    request = json.loads(REQUEST.read_bytes())
    source_manifest = (HERE / "SHA256SUMS").read_bytes()
    if sha(source_manifest) != request["source_manifest_sha256"]:
        raise ValueError("changed source manifest")
    pins = {}
    for line in source_manifest.decode().splitlines():
        digest, name = line.split("  ", 1)
        path = Path(name)
        if path.is_absolute() or ".." in path.parts or name in pins or not re.fullmatch(r"[a-f0-9]{64}", digest):
            raise ValueError(f"invalid manifest member: {name}")
        source = ROOT / path
        if source.parent == CUSTODY and source.name in CUSTODY_FILES:
            source = custody / source.name
        with source.open("rb") as stream:
            if hashlib.file_digest(stream, "sha256").hexdigest() != digest:
                raise ValueError(f"changed sealed member: {name}")
        pins[name] = digest
    from score import score

    sys.path.insert(0, str(custody))
    base = load(STOCK / "controller.py", "unfused_slow_controller")
    amendment = load(AMENDMENT / "controller.py", "unfused_slow_readable_controller")
    old = base.read(AMENDMENT / "inventory.json")
    prior_release = base.read(AMENDMENT / "evidence/proof/controller-release.json")
    base.require(prior_release["exit_status"] == 0 and prior_release["released_at"]
                 and prior_release["inventory_sha256"] == base.sha(AMENDMENT / "inventory.json")
                 and prior_release["request_sha256"] == base.sha(AMENDMENT / "request-proof.json")
                 and set(prior_release["raw_sha256"]) == {
                     f"00/diagnostic-two-{name}-{codec}/diagnostic-{engine}/raw.tar.gz"
                     for name, engine in base.ENGINES.items() for codec in ("pglz", "lz4")
                 }, "symmetric SLOW codec prerequisite is incomplete")
    base.require(all(base.sha(AMENDMENT / "evidence/proof" / name) == digest
                     for name, digest in prior_release["raw_sha256"].items()), "released codec proof bytes changed")
    base.require(request["id"] == ID + "-proof" and request["kind"] == "SETUP" and request["build"] is True
                 and request["groups"] == [0] and request["phases"] == ["diagnostic"]
                 and request["orders"] == {"0": list(EXPERIMENT)} and request["arms"] == len(EXPERIMENT)
                 and request["inventory_sha256"] == base.sha(AMENDMENT / "inventory.json")
                 and request["codec_proof_release_sha256"] == base.sha(AMENDMENT / "evidence/proof/controller-release.json"),
                 "qualification request differs")
    pricing = base.read(HERE / "pricing.json")
    raw_root = CAP71 / "evidence/native/sql-retry-timing"
    groups = set()
    for name, digest in pricing["release_sha256"].items():
        path = ROOT / name
        release = base.read(path)
        base.require(path.parent == raw_root / "controller-evidence" and base.sha(path) == digest
                     and release["exit_status"] == 0 and release["released_at"], "historical release differs")
        groups.update(release["groups"])
    base.require(groups == set(range(1, 9)) and len(pricing["blocks"]) == 8, "historical pricing needs all eight blocks")
    for block, lanes in pricing["blocks"].items():
        base.require(re.fullmatch(r"0[1-8]-slow\.[0-9]+", block) and set(lanes) == {"mesh", "go", "oban"},
                     "historical pricing arm differs")
        for engine, clocks in lanes.items():
            lane = f"measured-{engine}"
            raw = raw_root / "blocks" / block / lane / lane / "raw.tar.gz"
            base.require(base.sha(raw) == clocks["sha"], "historical pricing raw archive differs")
            with tarfile.open(raw, "r:gz") as archive:
                ledger = base.matrix.file_json(archive, "adapter/ledger.json")
            txn, = ledger["transactions"]
            terminal = max(job["settled_us"] for job in ledger["jobs"])
            base.require(clocks["submit"] == terminal - txn["submitted_us"]
                         and clocks["drain"] == terminal - txn["committed_us"], "historical price clock differs")

    base.ID, base.OUT, base.file_for = ID, OUT, lambda mode: REQUEST
    base.MODES = {"proof": ([0], request["requested_window_minutes"])}
    base.BUNDLE.update({"native-run.sh": HERE / "native-run.sh", "adapter.py": HERE / "adapter.py",
                        "stock_codec.py": AMENDMENT / "adapter.py"})
    bundle = {name: pins[str(path.relative_to(ROOT))] for name, path in base.BUNDLE.items()}
    for name in ("storage.py", "slow-river.sql"):
        base.BUNDLE[name] = custody / name
    base.require(request["bundle_sha256"] == bundle, "staged adapter bundle seal differs")
    inherited = old["composition_remote"]
    remote = str(base.matrix.REMOTE / f"{ID}-{request['source_manifest_sha256'][:16]}")
    lanes = {}
    old_bundle = base.read(AMENDMENT / "request-proof.json")["bundle_sha256"]
    for label, (source, mode) in EXPERIMENT.items():
        lane = deepcopy(old["lanes"][source])
        active = lane["inventory"]["engines"][lane["engine"]]
        base.require(active["argv"][1] == f"{inherited}/native-run.sh" and active["argv"][3] == "pglz",
                     "released PGLZ lane differs")
        for name, digest in old_bundle.items():
            base.require(active["verified_files"].pop(f"{inherited}/{name}") == digest, "old bundle pin differs")
        active["verified_files"].update({f"{remote}/{name}": digest for name, digest in bundle.items()})
        active["argv"][1], active["argv"][3] = f"{remote}/native-run.sh", mode
        base.matrix.check_inventory(lane["inventory"])
        lanes[label] = lane
    cfg = {**old, "lanes": lanes, "composition_remote": remote, "orders": {0: list(EXPERIMENT)}}
    base.score = lambda folder, label, cfg, phase: score(base, folder, label, cfg, phase)
    return base, amendment, cfg, bundle


def toolchain_binding(base, path):
    data = base.read(path)
    base.require(set(data) == {"request_sha256", "compiler", "compiler_sha256", "module_cache"}
                 and data["request_sha256"] == base.sha(REQUEST)
                 and re.fullmatch(r"[0-9a-f]{64}", data["compiler_sha256"])
                 and all(re.fullmatch(r"/[a-zA-Z0-9_./-]+", data[k]) and ".." not in Path(data[k]).parts
                         for k in ("compiler", "module_cache")), "native compiler/cache binding differs")
    return data


def receipt_and_bind(base, cfg, output):
    prior = base.read(CAP71 / "evidence/river-build.json")
    build = output / "built-river.tar.gz"
    with tarfile.open(build, "r:gz") as archive:
        base.require(archive.getnames() == ["compiler.txt", "build-info.txt", "elf-sha256.txt",
                                           "source-sha256.txt", "river-bench"]
                     and all(member.isfile() for member in archive.getmembers()), "native build archive is incomplete")
        compiler, info, elf_pins, sources, binary = [archive.extractfile(name).read() for name in archive.getnames()]
    elf = sha(binary)
    base.require(binary[:6] == b"\x7fELF\x02\x01" and binary[18:20] == b"\x3e\x00"
                 and elf != base.RIVER_SHA and prior["candidate_sha256"] == base.RIVER_SHA
                 and compiler == b"go version go1.26.4 linux/amd64\n"
                 and info.decode().splitlines() == prior["build_info"]
                 and elf_pins.decode().splitlines() == [f"{base.RIVER_SHA}  stock-elf", f"{elf}  river-bench"]
                 and sources == (output / "source-sha256.txt").read_bytes(), "exact SLOW rebuild cannot be bound")
    toolchain_binding(base, output / "toolchain.json")
    artifact = cfg["composition_remote"] + "-elf/river-bench"
    for lane in cfg["lanes"].values():
        if lane["engine"] != "go":
            continue
        spec = lane["inventory"]["engines"]["go"]
        base.require(spec["argv"][5] == base.RIVER and spec["artifact_sha256"] == base.RIVER_SHA,
                     "original River bridge inventory differs")
        if spec["argv"][3] != "stock":
            spec["argv"][5] = artifact
            spec["artifact_sha256"] = elf
            spec["verified_files"][artifact] = elf
        base.matrix.check_inventory(lane["inventory"])
    return dict(candidate_elf_sha256=elf, stock_elf_sha256=base.RIVER_SHA, artifact=artifact,
                build_archive_sha256=base.sha(build), compiled_sources_sha256=sha(sources),
                native_toolchain_binding_sha256=base.sha(output / "toolchain.json"))


def dispatch(base, amendment, cfg, bundle):
    queue = base.codec_packet.QUEUE
    base.require(base.read(REQUEST)["evidence_destination"] == str(OUT / "proof"),
                 "native dispatch must run from the controller's bound checkout")
    base.grant("proof", 600)
    binding = queue / (ID + ".toolchain.json")
    compiler = toolchain_binding(base, binding)
    amendment.stage_access(base, bundle)
    stage = base.stage

    def build_and_stage(cfg):
        base.grant("proof", 600)
        folder = OUT / "proof"
        (folder / "toolchain.json").write_bytes(binding.read_bytes())
        base.require(compiler == toolchain_binding(base, folder / "toolchain.json"), "toolchain binding changed")
        with tempfile.TemporaryDirectory(prefix="prepare-river-", dir=folder) as temp:
            source, checks = build_sources(base, Path(temp))
        (folder / "source-sha256.txt").write_text(checks)
        with (folder / "built-river.tar.gz").open("xb") as artifact, (folder / "native-build.log").open("xb") as log:
            script = base.codec_packet.original.HOST_IDLE + (HERE / "build-native.sh").read_text()
            args = [cfg["composition_remote"] + "-elf", compiler["compiler"], compiler["compiler_sha256"],
                    compiler["module_cache"], *cfg["cap71_design"]["host"], base.RIVER_SHA,
                    str(Path(cfg["postgres"]["prefix"]).parent / "river")]
            base.matrix.ssh(["bash", "-c", script, "--", *args], input=source, stdout=artifact, stderr=log,
                            timeout=480).check_returncode()
        base.write(folder / "build.json", receipt_and_bind(base, cfg, folder))
        base.grant("proof", 90)
        stage(cfg)

    base.stage = build_and_stage
    return base.dispatch("proof", cfg)


def replay(base, cfg, folder, release_path):
    doc = base.read(release_path)
    expected = {f"00/diagnostic-{label}/diagnostic-{lane['engine']}/raw.tar.gz"
                for label, lane in cfg["lanes"].items()}
    raws = {p.relative_to(folder).as_posix(): base.sha(p) for p in folder.rglob("raw.tar.gz")}
    build = receipt_and_bind(base, cfg, folder)
    bound = {name: base.sha(folder / name) for name in
             ("build.json", "built-river.tar.gz", "toolchain.json", "source-sha256.txt")}
    base.require(doc["id"] == ID + "-proof" and doc["exit_status"] == 0 and doc["released_at"]
                 and doc["request_sha256"] == base.sha(REQUEST) == base.read(folder / "grant.json")["packet_sha256"]
                 and set(raws) == expected and raws == doc["raw_sha256"] == base.read(folder / "raw-sha256.json")
                 and bound == doc["readiness_build_sha256"] and build == base.read(folder / "build.json"),
                 "canonical release does not bind the exact build and all diagnostic archives")
    for label, lane in cfg["lanes"].items():
        base.require(base.read(folder / "inventory" / label / "inventory.json") == lane["inventory"],
                     f"{label}: rebuilt or retained artifact inventory differs")
    base.OUT = folder.parent
    base.require(folder.name == "proof", "provide the released proof directory")
    rows = base.replay("proof", cfg)
    base.require(rows == base.read(folder / "replay.json") and rows["0"]["eligible"], "released proof did not reconcile")
    return dict(release_sha256=base.sha(release_path), build=build, qualification=rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "proof", "replay-proof"))
    parser.add_argument("--evidence", type=Path, default=OUT / "proof")
    parser.add_argument("--release", type=Path)
    args = parser.parse_args()
    installed = all((CUSTODY / name).is_file() for name in CUSTODY_FILES)
    with (nullcontext(CUSTODY) if installed else tempfile.TemporaryDirectory(prefix=".stock-custody-", dir=HERE)) as location:
        custody = Path(location)
        if not installed:
            for name in CUSTODY_FILES:
                path = (CUSTODY / name).relative_to(ROOT)
                data = subprocess.check_output(["git", "show", f"{CUSTODY_COMMIT}:{path}"], cwd=ROOT)
                (custody / name).write_bytes(data)
        base, amendment, cfg, bundle = validate_packet(custody)
        if args.action == "proof":
            return dispatch(base, amendment, cfg, bundle)
        if args.action == "replay-proof":
            release = args.release or base.released("proof")
            print(json.dumps(replay(base, cfg, args.evidence.resolve(), release), indent=2, sort_keys=True))
        else:
            print(json.dumps(dict(request_sha256=base.sha(REQUEST), native_bundle=bundle,
                                  host=cfg["cap71_design"]["host"], proof_order=list(EXPERIMENT)), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
