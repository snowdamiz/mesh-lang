"""Replay the exact seven SLOW diagnostics with the released build and staged code."""

import argparse
from contextlib import nullcontext
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
V1 = HERE.parent / "2026-11-25-slow-river-readiness"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
SUPPLY = QUEUE.parent / "controller-receipts/2026-11-25-slow-river-toolchain-supply"
OUT = HERE / "evidence"
REQUEST = HERE / "request-proof.json"
ID = HERE.name
OUTPUT_PINS = ("retained-inputs.json", "public-code-access.log", "retained-native-sha256.log")

ELF_ACCESS = """
set -euo pipefail
export LC_ALL=C
binary=$1
sha=$2
shift 2
test "$(readlink -e "$binary")" = "$binary"
test "$(stat -c '%a %u %F' -- "$binary")" = "555 0 regular file"
test "$(stat -c '%a %u %F' -- "${binary%/*}")" = "555 0 directory"
printf '%s  %s\n' "$sha" "$binary" | sha256sum --strict -c -
/usr/sbin/runuser -u nobody -- test -x "$binary"
"""


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(path.read_bytes())


def source():
    request = read(REQUEST)
    path = HERE / "SHA256SUMS"
    require(sha(path) == request["source_manifest_sha256"], "sibling source manifest differs")
    for line in path.read_text().splitlines():
        digest, filename = line.split("  ", 1)
        name = Path(filename)
        require(not name.is_absolute() and ".." not in name.parts and sha(ROOT / name) == digest,
                f"sealed sibling input differs: {filename}")
    require(request["custody_sha256"] == sha(HERE / "custody.json"), "retained custody seal differs")
    sys.path.insert(0, str(V1))
    spec = importlib.util.spec_from_file_location("slow_readiness_v1", V1 / "controller.py")
    original = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(original)
    return request, original


def bind_released_build(v1, base, cfg, pins, prior, supply):
    custody = read(HERE / "custody.json")
    prior_release = prior / "controller-release.json"
    supply_release = supply / "controller-release.json"
    release, supported = read(prior_release), read(supply_release)
    old_seal = sha(v1.REQUEST)
    grants = read(prior / "grant.json")
    require(sha(prior_release) == custody["prior_readiness_release_sha256"]
            and release["id"] == grants["id"] == custody["prior_readiness_id"] and release["released_at"]
            and release["exit_status"] == 1 and release["raw_sha256"] == {} and not any(prior.rglob("raw.tar.gz"))
            and release["request_sha256"] == release["grant"]["request_sha256"] == grants["packet_sha256"] == old_seal
            and sha(prior / "grant.json") == custody["prior_grant_sha256"],
            "prior build readiness release or zero-arm custody differs")
    require(sha(supply_release) == custody["supply_release_sha256"] and supported["released_at"]
            and supported["exit_status"] == 0 and supported["id"] == custody["supply_id"]
            and sha(supply / "native-receipt.json") == supported["native_receipt_sha256"]
            and sha(supply / "toolchain-binding.json") == supported["toolchain_binding_sha256"] == sha(prior / "toolchain.json"),
            "released toolchain custody differs")
    native = read(supply / "native-receipt.json")
    require(native["stock_elf_sha256"] == base.RIVER_SHA and native["host"] == cfg["cap71_design"]["host"]
            and native["supply_request_sha256"] == supported["request_sha256"]
            and native["toolchain_binding"] == read(prior / "toolchain.json"), "supply receipt belongs to another build")
    bound = {name: sha(prior / name) for name in custody["readiness_build_sha256"]}
    require(bound == release["readiness_build_sha256"] == custody["readiness_build_sha256"],
            "released build archive or its inputs differ")
    build = v1.receipt_and_bind(base, cfg, prior)
    require(build == read(prior / "build.json")
            and build["artifact"] == custody["retained_candidate_path"]
            and build["candidate_elf_sha256"] == custody["retained_candidate_elf_sha256"]
            and cfg["composition_remote"] == custody["retained_code_path"], "wrong retained candidate or code stage")
    return dict(prior_readiness_release_sha256=sha(prior_release), supply_release_sha256=sha(supply_release),
                readiness_build_sha256=bound, build=build, retained_code_path=cfg["composition_remote"],
                native_bundle=pins)


def check_request(base, request, cfg, retained):
    require(request["id"] == ID + "-proof" and request["kind"] == "SETUP"
            and request["build"] is False and request["timing_enrollment"] is False
            and request["arms"] == len(cfg["lanes"]) == 7 and request["groups"] == [0]
            and request["phases"] == ["diagnostic"] and request["orders"] == {"0": cfg["orders"][0]}
            and request["bundle_sha256"] == retained["native_bundle"]
            and request["retained_candidate_elf_sha256"] == retained["build"]["candidate_elf_sha256"]
            and request["prior_readiness_release_sha256"] == retained["prior_readiness_release_sha256"]
            and request["supply_release_sha256"] == retained["supply_release_sha256"], "build-free intake differs")
    base.ID, base.OUT, base.file_for = ID, OUT, lambda mode: REQUEST
    base.MODES = {"proof": ([0], request["requested_window_minutes"])}


def audit_lines(retained):
    names = sorted(retained["native_bundle"])
    return [retained["build"]["artifact"] + ": OK", *(name + ": OK" for name in names)]


def dispatch(base, amendment, request, cfg, retained, prior, supply):
    require(request["evidence_destination"] == str(OUT / "proof"), "run only from the controller-bound checkout")
    custody = read(HERE / "custody.json")
    for key, folder in (("prior_readiness_id", prior), ("supply_id", supply)):
        canonical = QUEUE / (custody[key] + ".released.json")
        require(canonical.read_bytes() == (folder / "controller-release.json").read_bytes(),
                f"archived release is not the controller's record: {key}")
    base.grant("proof", 120)

    def audit_existing_stage(config):
        base.grant("proof", 90)
        folder = OUT / "proof"
        path = config["composition_remote"]
        pins = retained["native_bundle"]
        names = sorted(pins)
        hashes = "".join(f"{pins[name]}  {name}\n" for name in names)
        binary = retained["build"]
        shell = base.codec_packet.original.HOST_IDLE + ELF_ACCESS + amendment.CODE_ACCESS
        log = folder / "public-code-access.log"
        with log.open("x") as stream:
            base.matrix.ssh(["bash", "-ec", shell, "--", binary["artifact"], binary["candidate_elf_sha256"], path, *names],
                            input=hashes, text=True, stdout=stream, stderr=subprocess.STDOUT,
                            timeout=45).check_returncode()
        require(log.read_text().splitlines() == audit_lines(retained), "retained stage checksum transcript differs")
        base.write(folder / "retained-inputs.json", retained)

    # dispatch verifies all retained engine/PG/ELF pins first. Its stage hook
    # now audits the existing stage and writes only to this sibling's folder.
    base.stage = audit_existing_stage
    return base.dispatch("proof", cfg)


def replay(base, cfg, retained, release_path):
    folder = OUT / "proof"
    release = read(release_path)
    require(sha(REQUEST) == release["request_sha256"] == read(folder / "grant.json")["packet_sha256"]
            and release["id"] == ID + "-proof" and release["released_at"] and release["exit_status"] == 0
            and release["retained_custody_sha256"] == {name: sha(folder / name) for name in OUTPUT_PINS}
            and read(folder / "retained-inputs.json") == retained
            and (folder / "public-code-access.log").read_text().splitlines() == audit_lines(retained),
            "sibling release or staged custody differs")
    expected = {f"00/diagnostic-{label}/diagnostic-{lane['engine']}/raw.tar.gz"
                for label, lane in cfg["lanes"].items()}
    raws = {p.relative_to(folder).as_posix(): sha(p) for p in folder.rglob("raw.tar.gz")}
    require(set(raws) == expected and raws == release["raw_sha256"] == read(folder / "raw-sha256.json"),
            "released sibling raw inventory differs")
    for label, lane in cfg["lanes"].items():
        require(read(folder / "inventory" / label / "inventory.json") == lane["inventory"],
                f"effective inventory differs: {label}")
    rows = base.replay("proof", cfg)
    require(rows == read(folder / "replay.json") and rows["0"]["eligible"], "seven-arm sibling proof did not reconcile")
    return dict(release_sha256=sha(release_path), retained_inputs=retained, qualification=rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "proof", "replay-proof"))
    parser.add_argument("--prior", type=Path, default=V1 / "evidence/proof")
    parser.add_argument("--supply", type=Path, default=SUPPLY)
    parser.add_argument("--release", type=Path, default=QUEUE / (ID + "-proof.released.json"))
    args = parser.parse_args()
    request, v1 = source()
    installed = all((v1.CUSTODY / name).is_file() for name in v1.CUSTODY_FILES)
    with (nullcontext(v1.CUSTODY) if installed else tempfile.TemporaryDirectory(prefix=".stock-custody-", dir=HERE)) as location:
        custody = Path(location)
        if not installed:
            for name in v1.CUSTODY_FILES:
                path = (v1.CUSTODY / name).relative_to(ROOT)
                (custody / name).write_bytes(subprocess.check_output(["git", "show", f"{v1.CUSTODY_COMMIT}:{path}"], cwd=ROOT))
        base, amendment, cfg, pins = v1.validate_packet(custody)
        retained = bind_released_build(v1, base, cfg, pins, args.prior, args.supply)
        check_request(base, request, cfg, retained)
        if args.action == "proof":
            return dispatch(base, amendment, request, cfg, retained, args.prior, args.supply)
        report = replay(base, cfg, retained, args.release) if args.action == "replay-proof" else {
            "request_sha256": sha(REQUEST), "proof_order": cfg["orders"][0], "retained_inputs": retained,
        }
        print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
