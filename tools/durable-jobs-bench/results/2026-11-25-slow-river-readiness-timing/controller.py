"""Controller-owned build-free timing for the released SLOW readiness sensitivity."""

import argparse
from copy import deepcopy
import fcntl
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tarfile

import criteria


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SUITE = HERE.parents[1]
REPLAY = HERE.parent / "2026-11-25-slow-river-readiness-replay"
V1 = HERE.parent / "2026-11-25-slow-river-readiness"
OUT = HERE / "evidence"
ID = HERE.name
LANES = ("mesh", "retained-river", "lazy-a", "warm-river", "lazy-b", "oban")
OFFSETS = (0, 1, 5, 2, 4, 3)
PHASES = ("warmup", "measured")
MODES = {"pilot": ([1, 2], 25), "confirm": ([3, 4, 5, 6], 40)}
CUSTODY_OUTPUTS = ("public-code-access.log", "retained-native-sha256.log", "retained-inputs.json")


INSTALL = """
set -euo pipefail
umask 077
old=$1
destination=$2
pins=$3
shift 3
test ! -e "$destination"
test ! -L "$destination"
stage=$(mktemp -d "${destination%/*}/.readiness-timing.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
cat > "$stage/native-run.sh"
for name in "$@"; do
    install -m 0444 -- "$old/$name" "$stage/$name"
done
chmod 0444 "$stage/native-run.sh"
(cd "$stage" && printf '%s' "$pins" | sha256sum --strict -c -)
chmod 0555 "$stage"
mv -T -- "$stage" "$destination"
"""


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def canonical_sha(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def file_for(mode):
    return HERE / f"request-{mode}.json"


def load_replay():
    spec = importlib.util.spec_from_file_location("replay_correction", REPLAY / "replay.py")
    replay = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(replay)
    return replay


def check_source():
    path = HERE / "SHA256SUMS"
    seal = sha(path)
    for line in path.read_text().splitlines():
        digest, filename = line.split("  ", 1)
        rel = Path(filename)
        require(not rel.is_absolute() and ".." not in rel.parts and sha(ROOT / rel) == digest,
                f"sealed timing source differs: {filename}")
    replay = load_replay()
    replay.manifest(REPLAY / "SOURCE-SHA256SUMS")
    requests = {mode: replay.read(file_for(mode)) for mode in MODES}
    require(all(request["source_manifest_sha256"] == seal for request in requests.values()),
            "timing source manifest seal differs")
    return replay, requests


def orders():
    start = dict(zip(OFFSETS, LANES, strict=True))
    result = {}
    for pair in range(3):
        row = [start[(pair + offset) % 6] for offset in OFFSETS]
        result[2 * pair + 1] = row
        result[2 * pair + 2] = row[::-1]
    require(tuple(result[1]) == LANES and all(len({result[k][i] for k in result}) == 6 for i in range(6)),
            "unbalanced timing order")
    return result


def design(base, cfg, retained, qualified, requests, replay):
    previous_stage = retained["retained_code_path"]
    old_pins = retained["native_bundle"]
    wrapper = HERE / "native-run.sh"
    original = (V1 / "native-run.sh").read_text()
    expect = original.replace("== diagnostic", "=~ ^(warmup|measured)$").replace(
        "peer|stock|cold|warm|underready", "peer|stock|cold|warm")
    require(wrapper.read_text() == expect, "timing wrapper changed more than phase admission")
    pins = {**old_pins, "native-run.sh": sha(wrapper)}
    remote = str(base.matrix.REMOTE / f"{ID}-{pins['native-run.sh'][:16]}")
    lanes = {}
    for label in LANES:
        lane = deepcopy(cfg["lanes"][label])
        spec = lane["inventory"]["engines"][lane["engine"]]
        require(spec["argv"][1] == previous_stage + "/native-run.sh", "prior staged wrapper differs")
        spec["argv"][1] = remote + "/native-run.sh"
        for name, digest in pins.items():
            require(spec["verified_files"].pop(previous_stage + "/" + name) == old_pins[name], "prior native source differs")
            spec["verified_files"][remote + "/" + name] = digest
        base.matrix.check_inventory(lane["inventory"])
        lanes[label] = lane
    inventory = {label: canonical_sha(lane["inventory"]) for label, lane in lanes.items()}
    plan = orders()
    replay.same(inventory, replay.read(HERE / "inventory-pins.json"), "timing inventories")
    for mode, request in requests.items():
        groups, minutes = MODES[mode]
        require(request["id"] == f"{ID}-{mode}" and request["kind"] == "TIMING" and request["build"] is False
                and request["groups"] == groups and request["requested_window_minutes"] == minutes
                and request["phases"] == list(PHASES) and request["arms"] == len(groups) * len(PHASES) * len(LANES)
                and request["orders"] == {str(i): plan[i] for i in groups}
                and request["bundle_sha256"] == pins and request["inventory_pins_sha256"] == sha(HERE / "inventory-pins.json")
                and request["qualified_replay_sha256"] == sha(REPLAY / "qualified.json")
                and request["proof_release_sha256"] == qualified["retained_release_sha256"]
                and request["candidate_elf_sha256"] == qualified["build"]["candidate_elf_sha256"]
                and request["pilot_request_sha256"] == (None if mode == "pilot" else sha(file_for("pilot"))),
                f"{mode}: sealed timing request differs")
    return {**cfg, "lanes": lanes, "orders": plan, "composition_remote": remote}, pins


def replay_groups(base, mode, cfg):
    rows = base.replay(mode, cfg)
    for block in rows.values():
        if not block["eligible"]:
            continue
        gate_errors = [error for phase in PHASES for error in criteria.gates(block["scores"], phase, LANES)]
        block["gate_errors"] = gate_errors
        block["eligible"] = not gate_errors
    return rows


def configure_base(base):
    base.ID, base.OUT = ID, OUT
    base.MODES = MODES.copy()
    old_score = base.score

    def score(folder, label, cfg, phase):
        result = old_score(folder, label, cfg, phase)
        engine = cfg["lanes"][label]["engine"]
        with tarfile.open(folder / f"{phase}-{engine}" / "raw.tar.gz") as raw:
            marks = base.matrix.file_json(raw, "adapter/startup-clock.json")
        launch = marks["adapter_start"]["epoch_ns"] // 1000
        require(launch < result["first_submit_us"] < result["last_settlement_us"], "adapter clocks crossed workload")
        result["adapter_start_to_terminal_ms"] = (result["last_settlement_us"] - launch) / 1000
        return result

    base.score = score


def grid(cfg, group):
    return [[phase, label] for phase in PHASES for label in cfg["orders"][group]]


def raw_hashes(path):
    return {p.relative_to(path).as_posix(): sha(p) for p in sorted(path.rglob("raw.tar.gz"))}


def unique_attempts(rows, previously_used):
    seen = set(previously_used)
    for block in rows.values():
        for row in block["scores"].values():
            require(row["attempt_id"] not in seen, "native attempt reused from a prior arm or proof")
            seen.add(row["attempt_id"])
    return seen


def log_pin_lines(cfg, pins, retained):
    return [retained["build"]["artifact"] + ": OK", *(name + ": OK" for name in sorted(pins))]


def custody_record(cfg, pins, retained, qualified):
    return dict(native_stage=cfg["composition_remote"], native_bundle=pins, build=retained["build"],
                retained_proof_release_sha256=qualified["retained_release_sha256"],
                qualified_replay_sha256=sha(REPLAY / "qualified.json"))


def audit(base, amendment, v2, mode, cfg, pins, retained, qualified, folder):
    host_idle = base.codec_packet.original.HOST_IDLE
    checks = "".join(f"{digest}  {name}\n" for name, digest in sorted(pins.items()))
    remote = cfg["composition_remote"]
    if mode == "pilot":
        cmd = ["bash", "-ec", host_idle + INSTALL, "--", retained["retained_code_path"], remote,
               checks, *(name for name in sorted(pins) if name != "native-run.sh")]
        with (HERE / "native-run.sh").open("rb") as wrapper, (folder / "timing-code-stage.log").open("xb") as log:
            base.matrix.ssh(cmd, stdin=wrapper, stdout=log, stderr=log, timeout=45).check_returncode()
    shell = host_idle + v2.ELF_ACCESS + amendment.CODE_ACCESS
    binary = retained["build"]
    cmd = ["bash", "-ec", shell, "--", binary["artifact"], binary["candidate_elf_sha256"], remote, *sorted(pins)]
    path = folder / "public-code-access.log"
    with path.open("x") as log:
        base.matrix.ssh(cmd, input=checks, text=True, stdout=log, stderr=log, timeout=45).check_returncode()
    require(path.read_text().splitlines() == log_pin_lines(cfg, pins, retained), "native stage audit differs")
    base.write(folder / "retained-inputs.json", custody_record(cfg, pins, retained, qualified))


def dispatch(base, amendment, v2, mode, request, cfg, pins, retained, qualified, used):
    folder = OUT / mode
    require(str(folder) == request["evidence_destination"], "run timing only from the bound controller checkout")
    released_proof = base.codec_packet.QUEUE / (v2.ID + "-proof.released.json")
    require(sha(released_proof) == qualified["retained_release_sha256"], "controller proof release differs")
    admit = lambda seconds: base.codec_packet.grant("TIMING", file_for(mode), seconds)
    admit(120)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        receipt = admit(120)
        folder.mkdir(parents=True)
        base.write(folder / "grant.json", receipt)
        base.matrix.ssh(["bash", "-c", base.codec_packet.original.HOST_IDLE], timeout=12).check_returncode()
        base.verify_retained_host_files(cfg, folder)
        admit(120)
        audit(base, amendment, v2, mode, cfg, pins, retained, qualified, folder)
        for label, lane in cfg["lanes"].items():
            admit(90)
            remote = base.matrix.REMOTE / f"{ID}-{mode}-{label}"
            dest = folder / "inventory" / label
            dest.mkdir(parents=True)
            base.write(dest / "inventory.json", lane["inventory"])
            with (dest / "transfer.log").open("xb") as log:
                for path in (SUITE / "worker.py", dest / "inventory.json"):
                    base.matrix.stage(path, remote, log)
        for group in MODES[mode][0]:
            base.matrix.ssh(["bash", "-c", base.codec_packet.original.HOST_IDLE], timeout=12).check_returncode()
            block = folder / f"{group:02d}"
            block.mkdir()
            plan = grid(cfg, group)
            base.write(block / "order.json", plan)
            for phase, label in plan:
                admit(90)
                stage = base.matrix.REMOTE / f"{ID}-{mode}-{label}"
                arm = block / f"{phase}-{label}"
                arm.mkdir()
                base.matrix.dispatch_arm(arm, stage, {"profile": "slow"}, phase, cfg["lanes"][label]["engine"])
        rows = replay_groups(base, mode, cfg)
        base.write(folder / "replay.json", rows)
        base.write(folder / "raw-sha256.json", raw_hashes(folder))
        unique_attempts(rows, used)
        base.matrix.ssh(["bash", "-c", base.codec_packet.original.HOST_IDLE], timeout=12).check_returncode()
        return 0 if all(block["eligible"] for block in rows.values()) else 1


def released(base, mode, cfg, pins, retained, qualified, replay, used):
    folder = OUT / mode
    release_path = base.codec_packet.QUEUE / f"{ID}-{mode}.released.json"
    release = replay.read(release_path)
    grant = replay.read(folder / "grant.json")
    expected = {f"{group:02d}/{phase}-{label}/{phase}-{cfg['lanes'][label]['engine']}/raw.tar.gz"
                for group in MODES[mode][0] for phase, label in grid(cfg, group)}
    raws = raw_hashes(folder)
    require(release["id"] == grant["id"] == f"{ID}-{mode}" and release["exit_status"] == 0 and release["released_at"]
            and release["request_sha256"] == grant["packet_sha256"] == sha(file_for(mode))
            and release["retained_custody_sha256"] == {name: sha(folder / name) for name in CUSTODY_OUTPUTS}
            and set(raws) == expected and raws == release["raw_sha256"] == replay.read(folder / "raw-sha256.json"),
            f"{mode}: released timing custody differs")
    replay.same(replay.read(folder / "retained-inputs.json"), custody_record(cfg, pins, retained, qualified), f"{mode}.custody")
    require((folder / "public-code-access.log").read_text().splitlines() == log_pin_lines(cfg, pins, retained),
            f"{mode}: retained timing audit differs")
    for label, lane in cfg["lanes"].items():
        replay.same(lane["inventory"], replay.read(folder / "inventory" / label / "inventory.json"), f"{mode}.{label}.inventory")
    rows = replay_groups(base, mode, cfg)
    require(all(row["eligible"] for row in rows.values()), f"{mode}: replay rejected one or more gates")
    replay.compare(rows, replay.read(folder / "replay.json"), MODES[mode][0], PHASES, LANES)
    return rows, sha(release_path), unique_attempts(rows, used)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "pilot", "replay-pilot", "confirm", "replay-confirm"))
    action = parser.parse_args().action
    replay, requests = check_source()
    with replay.context() as (base, amendment, v2, cfg, retained):
        qualified = replay.cleared(base, v2, cfg, retained)
        replay.same(qualified, replay.read(REPLAY / "qualified.json"), "qualified proof")
        cfg, pins = design(base, cfg, retained, qualified, requests, replay)
        configure_base(base)
        used = set(qualified["attempts"].values())
        if action == "check":
            report = dict(request_sha256={mode: sha(file_for(mode)) for mode in MODES},
                          inventory=sha(HERE / "inventory-pins.json"), orders=cfg["orders"],
                          candidate_elf_sha256=qualified["build"]["candidate_elf_sha256"])
        elif action == "pilot":
            return dispatch(base, amendment, v2, "pilot", requests["pilot"], cfg, pins, retained, qualified, used)
        else:
            pilot, pilot_sha, used = released(base, "pilot", cfg, pins, retained, qualified, replay, used)
            if action == "confirm":
                return dispatch(base, amendment, v2, "confirm", requests["confirm"], cfg, pins, retained, qualified, used)
            report = dict(pilot_release_sha256=pilot_sha, comparisons=criteria.study(pilot))
            if action == "replay-confirm":
                confirm, confirm_sha, _ = released(base, "confirm", cfg, pins, retained, qualified, replay, used)
                report = dict(pilot_release_sha256=pilot_sha, confirm_release_sha256=confirm_sha,
                              comparisons=criteria.study({**pilot, **confirm}))
        print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
