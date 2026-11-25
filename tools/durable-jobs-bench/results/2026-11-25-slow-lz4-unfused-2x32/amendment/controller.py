"""Continue the released unfused SLOW qualification under a new controller seal."""
import argparse
from copy import deepcopy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ORIGINAL = HERE.parent
ROOT = ORIGINAL.parents[3]
ID = ORIGINAL.name + "-readable-code-v1"
OUT = HERE / "evidence"
OLD_PROOF = ORIGINAL / "evidence/proof"
PREREQUISITES = {"proof": (), "pilot": ("proof",), "full": ("proof", "pilot")}


def read(path):
    return json.loads(path.read_bytes())


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def request_path(mode):
    return HERE / f"request-{mode}.json"


def original_controller():
    spec = importlib.util.spec_from_file_location("sealed_unfused_codec_controller", ORIGINAL / "controller.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def raw_hashes(folder):
    return {path.relative_to(folder).as_posix(): sha(path) for path in sorted(folder.rglob("raw.tar.gz"))}


def source_and_inventory():
    requests = {mode: read(request_path(mode)) for mode in PREREQUISITES}
    custody = read(HERE / "custody.json")
    expected = requests["proof"]["source_sha256"]
    for mode, packet in requests.items():
        require(packet["source_sha256"] == expected
                and packet["inventory_sha256"] == sha(HERE / "inventory.json")
                and packet["id"] == f"{ID}-{mode}" and packet["build"] is False
                and packet["requires"] == [requests[step]["id"] for step in PREREQUISITES[mode]]
                and packet["requires_request_sha256"] == {
                    requests[step]["id"]: sha(request_path(step)) for step in PREREQUISITES[mode]
                }
                and packet["parent_proof_release_sha256"] == custody["release_sha256"],
                f"{mode} request or prerequisite seal differs")
    for name, digest in expected.items():
        require(sha(ROOT / name) == digest, f"changed source or custody file {name}")

    base = original_controller()
    original = read(ORIGINAL / "inventory.json")
    frozen = read(ORIGINAL / "request-proof.json")
    stage_dir = original["composition_remote"] + "-readable-code-v1"
    base.BUNDLE["adapter.py"] = HERE / "adapter.py"
    pins = {name: sha(path) for name, path in base.BUNDLE.items()}
    require(pins == {**frozen["bundle_sha256"], "adapter.py": sha(HERE / "adapter.py")},
            "amendment changed another native source member")
    derived = deepcopy(original)
    derived["composition_remote"] = stage_dir
    inherited = original["composition_remote"]
    for label, lane in derived["lanes"].items():
        if label.startswith("two-"):
            spec = lane["inventory"]["engines"][lane["engine"]]
            require(spec["argv"][1] == f"{inherited}/native-run.sh", f"{label}: original wrapper differs")
            spec["argv"][1] = f"{stage_dir}/native-run.sh"
            for name, digest in pins.items():
                require(spec["verified_files"].pop(f"{inherited}/{name}") == frozen["bundle_sha256"][name],
                        f"{label}: original staged source differs: {name}")
                spec["verified_files"][f"{stage_dir}/{name}"] = digest
        base.matrix.check_inventory(lane["inventory"])
    require(derived == read(HERE / "inventory.json"), "derived inventory changed outside the code substitution")
    derived["orders"] = {int(group): order for group, order in derived["orders"].items()}
    proof = deepcopy(derived)
    proof["orders"][0] = [label for label in derived["orders"][0] if label.startswith("two-")]
    proof["lanes"] = {label: derived["lanes"][label] for label in proof["orders"][0]}
    config = {"proof": proof, "pilot": derived, "full": derived}

    base.ID, base.OUT, base.file_for = ID, OUT, request_path
    for mode, packet in requests.items():
        groups, minutes = base.MODES[mode]
        cfg = config[mode]
        require(packet["orders"] == {str(group): cfg["orders"][group] for group in groups}
                and packet["groups"] == groups and packet["requested_window_minutes"] == minutes
                and packet["phases"] == (["diagnostic"] if mode == "proof" else list(base.PHASES))
                and packet["arms"] == sum(len(base.arm_grid(mode, group, cfg)) for group in groups)
                and packet["bundle_sha256"] == pins and packet["evidence_destination"] == str(OUT / mode),
                f"{mode} dispatch contract differs")
    return base, config, original, custody, pins


def reuse_bridges(base, original, custody):
    release_path = OLD_PROOF / "controller-release.json"
    release = read(release_path)
    canonical = base.codec_packet.QUEUE / (release["id"] + ".released.json")
    require(sha(release_path) == sha(canonical) == custody["release_sha256"]
            and release["id"] == ORIGINAL.name + "-proof" and release["exit_status"] == 1
            and release["request_sha256"] == sha(ORIGINAL / "request-proof.json")
            == read(OLD_PROOF / "grant.json")["packet_sha256"]
            and release["raw_sha256"] == read(OLD_PROOF / "raw-sha256.json") == raw_hashes(OLD_PROOF),
            "original release or retained raw does not match custody")
    old = read(OLD_PROOF / "verified-replay.json")["0"]
    order = original["orders"]["0"]
    require(read(OLD_PROOF / "00/order.json") == [["diagnostic", label] for label in order]
            and not old["eligible"] and set(old["scores"]) == {"diagnostic-" + label for label in custody["bridges"]}
            and len(old["scores"]) == 4 and len(old["rejected"]) == 8,
            "original split or order differs")
    previous, selected = set(), {}
    for label in order:
        lane = original["lanes"][label]
        engine = lane["engine"]
        key = f"diagnostic-{label}"
        folder = OLD_PROOF / "00" / key
        raw = folder / f"diagnostic-{engine}" / "raw.tar.gz"
        with tarfile.open(raw, "r:gz") as archive:
            attempt = base.matrix.file_json(archive, "invocation.json")["attempt"]
            require(attempt not in previous, "original native attempt reused")
            previous.add(attempt)
        if label in custody["bridges"]:
            require(read(OLD_PROOF / "inventory" / label / "inventory.json") == lane["inventory"]
                    and release["raw_sha256"][raw.relative_to(OLD_PROOF).as_posix()] == custody["bridges"][label],
                    f"changed bridge cell {label}")
            row = base.score(folder, label, original, "diagnostic")
            require(row == old["scores"][key] and row["attempt_id"] == attempt,
                    f"released bridge replay differs for {label}")
            selected[key] = row
    return selected, previous


CODE_ACCESS = """set -o pipefail
test ! -L "$1"
test "$(stat -c '%a %u' -- "$1")" = "555 0"
exec /usr/sbin/runuser -u nobody -- env -i PATH=/usr/bin:/bin LC_ALL=C bash -ec '
    set -o pipefail
    cd -- "$1"
    test "$(pwd -P)" = "$1"
    shift
    test "$(find . -mindepth 1 -maxdepth 1 -printf "%f %y %u %m\\n" | sort)" = \
         "$(printf "%s f root 444\\n" "$@" | sort)"
    sha256sum --strict -c -
' -- "$@"
"""


def stage_access(base, pins):
    native_stage = base.stage
    retained_files = base.verify_retained_host_files

    def audit(cfg, folder):
        path = cfg["composition_remote"]
        inventory = "".join(f"{pins[name]}  {name}\n" for name in sorted(pins))
        with (folder / "public-code-access.log").open("x") as receipt:
            base.matrix.ssh(["bash", "-ec", CODE_ACCESS, "--", path, *sorted(pins)],
                            input=inventory, text=True, stdout=receipt, stderr=receipt,
                            timeout=30).check_returncode()

    def stage(cfg):
        path = cfg["composition_remote"]
        base.matrix.ssh(["mkdir", "-m", "0700", "--", path], timeout=12).check_returncode()
        native_stage(cfg)
        base.matrix.ssh(["chmod", "0555", "--", path], timeout=12).check_returncode()
        audit(cfg, OUT / "proof")

    def check_retained(cfg, folder):
        retained_files(cfg, folder)
        if folder.name != "proof":
            audit(cfg, folder)

    base.stage = stage
    base.verify_retained_host_files = check_retained


def released_replay(base, mode, cfg):
    path = base.released(mode)
    release = read(path)
    expected = {
        f"{group:02d}/{phase}-{label}/{phase}-{cfg['lanes'][label]['engine']}/raw.tar.gz"
        for group in base.MODES[mode][0] for phase, label in base.arm_grid(mode, group, cfg)
    }
    folder = OUT / mode
    require(path.read_bytes() == (folder / "controller-release.json").read_bytes()
            and set(release["raw_sha256"]) == expected
            and release["raw_sha256"] == raw_hashes(folder),
            f"{mode}: canonical release is missing from custody or differs from raw arm hashes")
    rows = base.replay(mode, cfg)
    require(rows == read(folder / "replay.json") and all(row["eligible"] for row in rows.values()),
            f"{mode}: controller and released replay differ or an arm failed")
    return rows, sha(path)


def combined_proof(base, cfg, bridges, old_attempts, custody):
    rows, release_sha = released_replay(base, "proof", cfg)
    fresh = rows["0"]["scores"]
    require(len(fresh) == 8 and not ({row["attempt_id"] for row in fresh.values()} & old_attempts),
            "amended proof reused an attempt or is incomplete")
    return dict(inherited_release_sha256=custody["release_sha256"], fresh_release_sha256=release_sha,
                inherited_bridge_scores=bridges, fresh_equal_topology_scores=fresh)


def unique_timing_attempts(rows, consumed):
    attempts = [arm["attempt_id"] for group in rows.values() for arm in group["scores"].values()]
    require(len(set(attempts)) == len(attempts) and not set(attempts).intersection(consumed),
            "timing reused a diagnostic or another timing arm")
    consumed.update(attempts)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "proof", "pilot", "full", "replay-proof",
                                          "replay-pilot", "replay-full", "score"))
    action = parser.parse_args().action
    base, config, original, custody, pins = source_and_inventory()
    bridges, old_attempts = reuse_bridges(base, original, custody)
    if action == "check":
        print(json.dumps({mode: sha(request_path(mode)) for mode in PREREQUISITES}, sort_keys=True))
        return 0
    if action == "proof":
        stage_access(base, pins)
        return base.dispatch(action, config[action])
    proof = combined_proof(base, config["proof"], bridges, old_attempts, custody)
    if action == "replay-proof":
        print(json.dumps(proof, sort_keys=True, indent=2))
        return 0
    old_attempts.update(row["attempt_id"] for row in proof["fresh_equal_topology_scores"].values())
    if action == "pilot":
        stage_access(base, pins)
        return base.dispatch(action, config[action])
    pilot, pilot_sha = released_replay(base, "pilot", config["pilot"])
    unique_timing_attempts(pilot, old_attempts)
    if action == "full":
        stage_access(base, pins)
        return base.dispatch(action, config[action])
    if action == "replay-pilot":
        print(json.dumps(pilot, sort_keys=True, indent=2))
        return 0
    full, full_sha = released_replay(base, "full", config["full"])
    unique_timing_attempts(full, old_attempts)
    if action == "replay-full":
        print(json.dumps(full, sort_keys=True, indent=2))
        return 0
    rows = {**pilot, **full}
    pairs = base.paired(rows)
    require(len(pairs) == 6, "complete six matched rotations required")
    print(json.dumps(dict(selected=rows, per_order=pairs, distributions=base.distribution(list(pairs.values())),
                          proof_release_sha256=proof["fresh_release_sha256"],
                          pilot_release_sha256=pilot_sha, full_release_sha256=full_sha), sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
