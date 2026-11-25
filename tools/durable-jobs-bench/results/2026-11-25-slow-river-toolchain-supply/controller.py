"""Sole-controller intake for a separately authorized SLOW Go supply SETUP."""

import argparse
import base64
from datetime import datetime, timedelta, timezone
import fcntl
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import zipfile


HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
SUITE = RESULTS.parent
ROOT = SUITE.parents[1]
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
CACHE = ROOT / "target/linode-river/cache/mod"
EVIDENCE = QUEUE.parent / "controller-receipts" / HERE.name
REQUEST = HERE / "request.json"
PROOF = RESULTS / "2026-11-25-slow-river-readiness/request-proof.json"
ORIGINAL = RESULTS / "2026-11-24-ordinary-slow-equal-topology-cap71/evidence/river-build.json"
MATCHED = RESULTS / "2026-11-25-slow-lz4-unfused-2x32/amendment/inventory.json"
SOURCE_COMMIT = "1617e257f26ec7eb3b5996406d0993918d482226"
SOURCE_FILES = ("db.go", "evidence.go", "main.go", "worker.go", "go.mod", "go.sum")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read(path):
    return json.loads(path.read_bytes())


def pin(data):
    return dict(sha256=sha(data), size=len(data))


def go_hash(entries):
    summary = hashlib.sha256()
    for name, content in entries:
        require("\n" not in name, f"invalid Go archive name: {name!r}")
        summary.update(f"{sha(content)}  {name}\n".encode())
    return "h1:" + base64.b64encode(summary.digest()).decode()


def sealed():
    request = read(REQUEST)
    manifest = (HERE / "SHA256SUMS").read_bytes()
    require(request["id"] == HERE.name and request["kind"] == "SETUP" and request["build"] is True
            and request["arms"] == 0 and request["timing"] is False
            and request["source_manifest_sha256"] == sha(manifest), "supply request or source manifest differs")
    for line in manifest.decode().splitlines():
        digest, name = line.split("  ", 1)
        path = Path(name)
        require(not path.is_absolute() and ".." not in path.parts and sha((ROOT / path).read_bytes()) == digest,
                f"sealed file differs: {name}")
    require(request["proof_request_sha256"] == sha(PROOF.read_bytes())
            and request["inventory_sha256"] == sha((HERE / "module-inventory.json").read_bytes())
            and request["sdk_pin_sha256"] == sha((HERE / "sdk.json").read_bytes()), "upstream contract differs")
    return request


def inputs(cache):
    inventory = read(HERE / "module-inventory.json")
    prior = read(ORIGINAL)
    require(inventory["source_commit"] == SOURCE_COMMIT, "frozen source commit differs")
    source = {}
    for name in SOURCE_FILES:
        path = (SUITE / "go" / name).relative_to(ROOT).as_posix()
        body = subprocess.check_output(["git", "show", f"{SOURCE_COMMIT}:{path}"], cwd=ROOT)
        require(sha(body) == prior["stock_source"][path] == inventory["frozen_source_sha256"][name],
                f"frozen source bytes differ: {name}")
        source[name] = body
    sums = {(m, v): h for m, v, h in map(str.split, source["go.sum"].decode().splitlines())}
    go_mod = source["go.mod"].decode()
    roots = re.findall(r"^\s*([^\s()]+\.[^\s()]+)\s+(v\S+)\s*(?://.*)?$", go_mod, re.M)
    deps = [dict(module=m, version=v, h1=h) for _, _, m, v, h in
            (line.split("\t") for line in prior["build_info"] if line.startswith("\tdep\t"))]
    require(set(roots) == {(d["module"], d["version"]) for d in deps}
            and deps == inventory["linked_modules"]
            and all(sums[d["module"], d["version"]] == d["h1"] for d in deps),
            "go.mod, go.sum and retained build graph disagree")
    files = {}
    modfiles = {f"cache/download/{m}/@v/{v.removesuffix('/go.mod')}.mod": checksum
                for (m, v), checksum in sums.items() if v.endswith("/go.mod")}
    zip_stems = {f"cache/download/{d['module']}/@v/{d['version']}": d["h1"] for d in deps}
    archive_files = {stem + suffix for stem in zip_stems for suffix in (".zip", ".ziphash")}
    require(set(inventory["files"]) == archive_files | modfiles.keys()
            and len(modfiles) == inventory["go_mod_entries"], "cache allowlist differs")
    root = cache.resolve(strict=True)
    for name, expected in inventory["files"].items():
        path = root / name
        require(path.resolve(strict=True) == path and path.is_file() and path.stat().st_size == expected["size"],
                f"unexpected module cache file: {name}")
        body = path.read_bytes()
        require(pin(body) == expected, f"module cache bytes differ: {name}")
        files["mod/" + name] = body
    require(sum(map(len, files.values())) == inventory["bytes"], "module payload size differs")
    for name, checksum in modfiles.items():
        require(go_hash([("go.mod", files["mod/" + name])]) == checksum, f"Go module metadata differs: {name}")
    for stem, checksum in zip_stems.items():
        path = "mod/" + stem
        with zipfile.ZipFile(io.BytesIO(files[path + ".zip"])) as archive:
            names = archive.namelist()
            require(names and len(names) == len(set(names)), f"invalid archive inventory: {stem}")
            actual = go_hash((name, archive.read(name)) for name in sorted(names))
        require(actual == files[path + ".ziphash"].decode().strip() == checksum, f"Go archive checksum differs: {stem}")
    before, after = (prior[key].encode() for key in ("before", "after"))
    require(source["main.go"].count(before) == 1, "retained SLOW profile anchor differs")
    source["main.go"] = source["main.go"].replace(before, after)
    require(sha(source["main.go"]) == prior["patched_main_sha256"], "cap-71 stock source differs")
    files.update({"stock/" + name: body for name, body in source.items()})
    return prior, inventory, files


def grant(request):
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)", (QUEUE / "STATUS.md").read_text(), re.M)
    require(events and events[-1][:2] == ("SETUP", request["id"]), "no active controller supply SETUP")
    kind, identifier, start, end = events[-1]
    now = datetime.now(timezone.utc)
    require(datetime.fromisoformat(start) <= now < datetime.fromisoformat(end) - timedelta(seconds=600),
            "insufficient exclusive supply window")
    raw = REQUEST.read_bytes()
    require((QUEUE / f"{identifier}.json").read_bytes() == raw, "controller inbox supply bytes differ")
    return dict(id=identifier, kind=kind, from_utc=start, until_utc=end, request_sha256=sha(raw))


def stage(request, prior, files, sdk_path):
    sdk = read(HERE / "sdk.json")
    require(sdk_path.is_file() and sdk_path.stat().st_size == sdk["size"], "official SDK archive is absent or wrong-sized")
    body = sdk_path.read_bytes()
    require(pin(body) == {k: sdk[k] for k in ("sha256", "size")}, "official SDK archive digest differs")
    files[sdk["filename"]] = body
    with io.BytesIO() as transport:
        with tarfile.open(fileobj=transport, mode="w") as archive:
            for name, content in sorted(files.items()):
                member = tarfile.TarInfo(name)
                member.mode = 0o600
                member.size = len(content)
                archive.addfile(member, io.BytesIO(content))
        payload = transport.getvalue()
    require(str(EVIDENCE) == request["evidence_destination"], "supply must run in the controller checkout")
    grant(request)
    sys.path.insert(0, str(SUITE))
    import run as matrix

    matched = read(MATCHED)
    retained = matched["lanes"]["two-river-pglz"]["inventory"]["engines"]["go"]
    stock_path = retained["argv"][5]
    require(retained["artifact_sha256"] == retained["verified_files"][stock_path] == prior["candidate_sha256"],
            "retained stock ELF pin differs")
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        admission = grant(request)
        data = dict(host=matched["cap71_design"]["host"], storage=retained["postgres_storage"],
                    retained_stock_elf=stock_path, stock_sha256=prior["candidate_sha256"],
                    sdk_sha256=sdk["sha256"], inventory_sha256=request["inventory_sha256"],
                    proof_request_sha256=request["proof_request_sha256"], build_info=prior["build_info"],
                    supply_request_sha256=admission["request_sha256"], payload={n: pin(b) for n, b in files.items()})
        EVIDENCE.mkdir()
        matrix.write_json(EVIDENCE / "grant.json", admission)
        matrix.write_json(EVIDENCE / "native-input-pins.json", data)
        command = ["/usr/bin/python3", "-I", "-B", "-c", (HERE / "native.py").read_text(), json.dumps(data)]
        with (EVIDENCE / "native-receipt.json").open("xb") as output, (EVIDENCE / "native.log").open("xb") as log:
            matrix.ssh(command, input=payload, stdout=output, stderr=log, timeout=480).check_returncode()
        receipt = read(EVIDENCE / "native-receipt.json")
        binding = receipt["toolchain_binding"]
        target = "/var/lib/durable-jobs-bench/native/toolchain/slow-river-readiness-go1.26.4-" + admission["request_sha256"][:16]
        require(receipt["host"] == data["host"] and receipt["stock_elf_sha256"] == data["stock_sha256"]
                and receipt["sdk_sha256"] == data["sdk_sha256"] and receipt["stock_build_info"] == prior["build_info"]
                and receipt["compiler_version"] == "go version go1.26.4 linux/amd64"
                and receipt["supply_request_sha256"] == admission["request_sha256"]
                and receipt["module_inventory_sha256"] == request["inventory_sha256"]
                and set(binding) == {"request_sha256", "compiler", "compiler_sha256", "module_cache"}
                and binding["request_sha256"] == request["proof_request_sha256"]
                and re.fullmatch(r"[a-f0-9]{64}", binding["compiler_sha256"])
                and binding["compiler"] == f"{target}/go/bin/go" and binding["module_cache"] == f"{target}/mod",
                "native receipt does not bind to this supply and exact proof requests")
        matrix.write_json(EVIDENCE / "toolchain-binding.json", binding)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "stage"))
    parser.add_argument("--cache", type=Path, default=CACHE)
    parser.add_argument("--sdk", type=Path)
    args = parser.parse_args()
    if (args.action == "stage") != (args.sdk is not None):
        parser.error("stage requires an already supplied --sdk file")
    request = sealed()
    prior, inventory, files = inputs(args.cache)
    if args.action == "stage":
        stage(request, prior, files, args.sdk)
    else:
        print(json.dumps(dict(request_sha256=sha(REQUEST.read_bytes()), cache_files=len(inventory["files"]),
                              cache_bytes=inventory["bytes"], linked_modules=len(inventory["linked_modules"])),
                         sort_keys=True))


if __name__ == "__main__":
    main()
