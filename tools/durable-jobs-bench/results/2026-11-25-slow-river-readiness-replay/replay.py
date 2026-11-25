"""Read-only replay of the sealed SLOW readiness proof with explicit PID keys."""

from contextlib import contextmanager, nullcontext
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import tempfile


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
V2 = HERE.parent / "2026-11-25-slow-river-readiness-retained-v2"
EVIDENCE = HERE / "evidence"
PROOF = EVIDENCE / "retained/proof"
V2_RELEASE_SHA = "da0daba87bc31f322f5c91f339cf29cfe3a00b6712092b16669ad4e36950d0d7"
DISPATCH_REPLAY_SHA = "cafa0cdcc45ecad903f4517e147e5d7a6c2fb1bbf68f3a90ac6de98f83a9f2c9"
GRANT_SHA = "24b32b596b58acb3ae382c03cc7227c7a2c545f660aa1653364f00eb8e3e1ce3"
FIELD = "ready_detached_listeners"
REBUILT = ("lazy-a", "lazy-b", "warm-river", "underready-river")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key!r}")
        result[key] = value
    return result


def read(path):
    return json.loads(path.read_bytes(), object_pairs_hook=unique_object)


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def manifest(path):
    files = {}
    for line in path.read_text().splitlines():
        digest, filename = line.split("  ", 1)
        rel = Path(filename)
        require(not rel.is_absolute() and ".." not in rel.parts and filename not in files
                and sha(ROOT / rel) == digest, f"manifest member differs: {filename}")
        files[filename] = digest
    return files


def same(actual, expected, path):
    require(type(actual) is type(expected), f"{path}: expected {type(expected).__name__}, got {type(actual).__name__}")
    if isinstance(expected, dict):
        require(all(type(k) is str for k in actual) and actual.keys() == expected.keys(), f"{path}: unequal keys")
        for key in expected:
            same(actual[key], expected[key], f"{path}.{key}")
    elif isinstance(expected, list):
        require(len(actual) == len(expected), f"{path}: unequal lengths")
        for i, (a, b) in enumerate(zip(actual, expected, strict=True)):
            same(a, b, f"{path}[{i}]")
    else:
        require(actual == expected, f"{path}: unequal values")


def canonical_listener_keys(actual, expected, path):
    require(type(actual) is type(expected) is dict and len(actual) == len(expected) == 2, f"{path}: listener count or type")
    names = list(actual.values())
    require(all(type(name) is str and re.fullmatch(r"river:parity_[a-f0-9]{16}:runner-[01]", name) for name in names)
            and len({name.rsplit(":", 1)[0] for name in names}) == 1
            and {name.rsplit(":", 1)[1] for name in names} == {"runner-0", "runner-1"}, f"{path}: listener identities")
    for key in expected:
        require(type(key) is str and re.fullmatch(r"[1-9][0-9]{0,9}", key) and int(key) <= 2**31 - 1,
                f"{path}: noncanonical archived PID {key!r}")
    result = {}
    for pid, name in actual.items():
        require(type(pid) is int and 0 < pid <= 2**31 - 1, f"{path}: invalid replay PID {pid!r}")
        key = str(pid)
        require(key not in result, f"{path}: colliding replay PID {key}")
        result[key] = name
    same(result, expected, path)
    return result


def compare(rows, recorded, blocks, phases, lanes):
    for group in blocks:
        for phase in phases:
            for lane in lanes:
                if lane not in REBUILT:
                    continue
                label = f"{phase}-{lane}"
                actual = rows[str(group)]["scores"][label]
                expected = recorded[str(group)]["scores"][label]
                actual[FIELD] = canonical_listener_keys(actual[FIELD], expected[FIELD], f"{group}.{label}.{FIELD}")
    same(rows, recorded, "replay")


@contextmanager
def context():
    v2 = load(V2 / "controller.py", "retained_readiness")
    request, v1 = v2.source()
    installed = all((v1.CUSTODY / name).is_file() for name in v1.CUSTODY_FILES)
    with (nullcontext(v1.CUSTODY) if installed else tempfile.TemporaryDirectory(prefix=".custody-", dir=HERE)) as location:
        custody = Path(location)
        if not installed:
            for name in v1.CUSTODY_FILES:
                path = (v1.CUSTODY / name).relative_to(ROOT)
                (custody / name).write_bytes(subprocess.check_output(["git", "show", f"{v1.CUSTODY_COMMIT}:{path}"], cwd=ROOT))
        base, amendment, cfg, pins = v1.validate_packet(custody)
        retained = v2.bind_released_build(v1, base, cfg, pins, EVIDENCE / "original-build", EVIDENCE / "supply")
        v2.check_request(base, request, cfg, retained)
        yield base, amendment, v2, cfg, retained


def cleared(base, v2, cfg, retained):
    paths = manifest(HERE / "EVIDENCE-SHA256SUMS")
    require(set(paths) == {p.relative_to(ROOT).as_posix() for p in EVIDENCE.rglob("*") if p.is_file()},
            "archived proof histories have extra or missing files")
    release_path = PROOF / "controller-release.json"
    release = read(release_path)
    grant = read(PROOF / "grant.json")
    require(sha(release_path) == V2_RELEASE_SHA and sha(PROOF / "grant.json") == GRANT_SHA
            and sha(PROOF / "replay.json") == DISPATCH_REPLAY_SHA
            and release["request_sha256"] == grant["packet_sha256"] == sha(v2.REQUEST)
            and release["id"] == grant["id"] == v2.ID + "-proof" and release["phase"] == "setup"
            and release["released_at"] and release["exit_status"] == 0
            and release["retained_custody_sha256"] == {name: sha(PROOF / name) for name in v2.OUTPUT_PINS}
            and read(PROOF / "retained-inputs.json") == retained
            and (PROOF / "public-code-access.log").read_text().splitlines() == v2.audit_lines(retained),
            "released seven-arm proof custody differs")
    expected = {f"00/diagnostic-{label}/diagnostic-{lane['engine']}/raw.tar.gz"
                for label, lane in cfg["lanes"].items()}
    raws = {p.relative_to(PROOF).as_posix(): sha(p) for p in PROOF.rglob("raw.tar.gz")}
    require(set(raws) == expected and raws == release["raw_sha256"] == read(PROOF / "raw-sha256.json"),
            "released seven-arm raw inventory differs")
    for label, lane in cfg["lanes"].items():
        same(lane["inventory"], read(PROOF / "inventory" / label / "inventory.json"), f"inventory.{label}")
    base.OUT = PROOF.parent
    rows = base.replay("proof", cfg)
    require(rows["0"]["eligible"] is True and rows["0"]["rejected"] == {}
            and set(rows["0"]["scores"]) == {"diagnostic-" + label for label in cfg["lanes"]},
            f"proof gate rejected: {rows['0']['rejected']}")
    compare(rows, read(PROOF / "replay.json"), (0,), ("diagnostic",), cfg["lanes"])
    scores = rows["0"]["scores"]
    bad = scores["diagnostic-underready-river"]
    require(bad["ready_physical_clients"] == 66 and "at ready, observed 33/32" in bad["physical_readiness_rejection"]
            and "first_submit_us" not in bad, "expected no-submit shortage did not qualify")
    return dict(qualified=True, source_manifest_sha256=sha(HERE / "SOURCE-SHA256SUMS"),
                archived_evidence_manifest_sha256=sha(HERE / "EVIDENCE-SHA256SUMS"),
                build=retained["build"], original_readiness_release_sha256=retained["prior_readiness_release_sha256"],
                supply_release_sha256=retained["supply_release_sha256"], retained_release_sha256=V2_RELEASE_SHA,
                dispatch_replay_sha256=DISPATCH_REPLAY_SHA, raw_sha256=raws,
                normalized_listener_pid_lanes=list(REBUILT),
                attempts={label: row["attempt_id"] for label, row in scores.items()},
                readiness={label: row["ready_physical_clients"] for label, row in scores.items()
                           if "ready_physical_clients" in row},
                underready=bad["physical_readiness_rejection"])


def main():
    manifest(HERE / "SOURCE-SHA256SUMS")
    with context() as (base, _, v2, cfg, retained):
        print(json.dumps(cleared(base, v2, cfg, retained), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
