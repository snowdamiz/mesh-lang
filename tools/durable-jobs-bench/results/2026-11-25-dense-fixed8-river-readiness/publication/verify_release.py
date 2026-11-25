"""Independently verify the dense readiness release from a relocated Git export."""

from datetime import datetime, timezone
import gzip
import json
from pathlib import Path, PurePosixPath
import runpy
import sys
import tarfile

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
# Load by path so the factorial can still import its own controller dependency.
sealed = runpy.run_path(str(HERE / "controller.py"))
build, run, metrics, factorial = (sealed[name] for name in ("build", "run", "metrics", "factorial"))
OUT = sealed["OUT"]
AUDIT = HERE / "evidence/release-audit.json"


def json_bytes(value):
    return (json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()


def verified_inputs():
    receipt = build.read(build.RECEIPT)
    image = gzip.decompress(build.BINARY.read_bytes())
    assert image[:6] == b"\x7fELF\x02\x01" and image[18:20] == b"\x3e\x00"
    assert build.sha(image) == receipt["candidate_elf"]
    assert run.digest(build.BINARY) == receipt["compressed_sha256"]
    assert sealed["packet"](receipt) == build.read(sealed["PACKET"])
    assert json_bytes(sealed["history"]()) == sealed["HISTORY"].read_bytes()
    return sealed["config"](receipt)


def observe_arm(folder, row, start, released, attempts):
    raw = folder / "raw.tar.gz"
    request = build.read(folder / "request.json")
    dispatch = build.read(folder / "dispatch.json")
    attempt = request["attempt"]
    assert attempt not in attempts
    attempts.add(attempt)
    created = datetime.fromisoformat(request["created"])
    finished = datetime.fromisoformat(dispatch["finished"])
    assert start <= created <= finished <= released and dispatch["exit_code"] == 0
    with tarfile.open(raw) as archive:
        for member in archive.getmembers():
            path = PurePosixPath(member.name)
            assert member.isfile() and not path.is_absolute() and ".." not in path.parts
        status = run.file_json(archive, "status.json")
        host_finish = datetime.fromtimestamp(status["finished_us"] / 1e6, timezone.utc)
        assert status["exit_code"] == 0 and start <= host_finish <= released
        ledger = archive.extractfile("adapter/ledger.json").read()
        assert row["ledger_sha256"] == build.sha(ledger)
    digest = run.digest(raw)
    assert row["raw_sha256"] == digest
    return dict(attempt=attempt, group_path=str(raw.relative_to(OUT)), raw_sha256=digest,
                ledger_sha256=row["ledger_sha256"])


def verify():
    cfg = verified_inputs()
    packet_seal = run.digest(sealed["PACKET"])
    releases = {}
    rows = {}
    by_group = {}
    attempts = set()
    previous_release = None
    for stage, (trials, minutes) in sealed["STAGES"].items():
        release_path = sealed["release_path"](stage)
        release = build.read(release_path)
        grants = tuple(OUT.glob(f"grant-{stage}-*.json"))
        grant_file, = grants
        grant = build.read(grant_file)
        assert grant["id"] == release["id"] and grant["request_sha256"] == packet_seal
        start, end = (datetime.fromisoformat(grant[name]) for name in ("start", "end"))
        released = datetime.fromisoformat(release["released_at"])
        opened = datetime.fromisoformat(release["started"])
        assert start <= opened < released <= end and (end - start).total_seconds() == minutes * 60
        assert previous_release is None or previous_release < start
        previous_release = released
        if stage != "setup":
            controller_copy, = [path for path in (OUT / "controller").glob("controller-timing-*.json")
                                if build.read(path)["id"] == release["id"]]
            assert release_path.read_bytes() == controller_copy.read_bytes()
        stagings = tuple((OUT / "staging").glob(f"{stage}-*"))
        staging, = stagings
        assert {path.name for path in staging.iterdir()} == set(sealed["LANES"])
        for lane in sealed["LANES"]:
            assert build.read(staging / lane / "inventory.json") == cfg["lanes"][lane]["inventory"]
        blocks = sealed["require_phase"](stage, cfg)
        for trial in trials:
            group = trial["group"]
            path, = (OUT / "blocks").glob(f"{run.block_prefix(trial)}.*")
            by_group[group] = [observe_arm(folder, blocks[group][phase][lane], start, released, attempts)
                               for phase, lane, _, folder in factorial.grouped.matrix.paths(path, cfg, trial)]
        releases[stage] = release
        rows.update(blocks)
        measured = {group: row["measured"] for group, row in rows.items() if "measured" in row}
        replay = dict(dispositions={name: releases.get(name, "no release receipt") for name in sealed["STAGES"]},
                      groups=rows, paired=metrics.comparisons(measured) if measured else None)
        assert json_bytes(replay) == (OUT / f"replay-{stage}.json").read_bytes()
    found = {str(raw.relative_to(OUT)) for raw in OUT.rglob("raw.tar.gz")}
    assert found == {arm["group_path"] for arms in by_group.values() for arm in arms}
    assert len(found) == len(attempts) == 52 and len(by_group[5]) == 8
    assert [len(by_group[n]) for n in range(7)] == [4, 8, 8, 8, 8, 8, 8]

    mesh_health = {}
    for group in range(1, 7):
        path, = (OUT / "blocks").glob(f"{group:02d}-noop.*")
        row = factorial.score_arm(path / "measured-combined/measured-mesh", "measured", "combined", cfg,
                                  factorial.grouped, factorial.logical, factorial.POLICIES)
        mesh_health[group] = {name: row[name] for name in (
            "cpu_steal_percent", "cpu_iowait_percent", "host_cpu_ms", "pg_cpu_ms", "terminal_xids")}
    return dict(packet_sha256=packet_seal, raw_by_group=by_group, mesh_health_by_group=mesh_health,
                releases={stage: dict(release_sha256=run.digest(sealed["release_path"](stage)),
                                      replay_sha256=run.digest(OUT / f"replay-{stage}.json"),
                                      release=release)
                          for stage, release in releases.items()},
                native_sha256={str(path.relative_to(OUT)): run.digest(path)
                               for path in sorted(OUT.rglob("*")) if path.is_file()})


if __name__ == "__main__":
    result = json_bytes(verify())
    if sys.argv[1:] == ["--record"]:
        with AUDIT.open("xb") as stream:
            stream.write(result)
    else:
        assert sys.argv[1:] == [] and result == AUDIT.read_bytes()
        print("Verified three exact exit-0 releases, three byte-identical replays, and 52 unique raw archives.")
