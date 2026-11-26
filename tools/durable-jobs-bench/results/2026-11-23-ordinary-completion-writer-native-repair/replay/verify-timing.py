"""Replay all available matched retry arms from raw files in the checkout."""
from datetime import datetime
import json
from pathlib import Path

from controller import ID, PACKET, REQUESTS, pins, qualified
from timing import matrix, plan, run


def verify():
    packet, earlier = pins()
    qualification = qualified(packet, earlier)
    release = run.read_json(REQUESTS / f"{ID}.released.json")
    assert release["id"] == ID and release["exit_status"] == 0 and release["released_at"]
    timing_releases, granted_groups = [], set()
    for start, end, command in ((1, 2, "timing_command"), (3, 5, "remaining_timing_command")):
        path = REQUESTS / f"{ID}-groups-{start}-{end}.released.json"
        if not path.is_file():
            continue
        timing_release = run.read_json(path)
        assert timing_release["id"] == ID and timing_release["group_command"] == command
        assert timing_release["groups"] == list(range(start, end + 1))
        assert timing_release["exit_status"] == timing_release["attempts"][-1] == 0
        assert (datetime.fromisoformat(release["released_at"]) <= datetime.fromisoformat(timing_release["started"])
                < datetime.fromisoformat(timing_release["released_at"]))
        assert timing_release in [run.read_json(path) for path in (PACKET / "evidence").glob(
            "controller-timing-*.json")]
        timing_releases.append(timing_release)
        granted_groups.update(timing_release["groups"])
    series = PACKET / "evidence/native-timing" / ID
    config = run.read_json(qualification)
    expected = list(plan(config["profiles"], config["lanes"]))
    matched, missing = [], []
    if (series / "series.json").is_file():
        manifest = run.read_json(series / "series.json")
        assert manifest["config"] == config and manifest["plan"] == expected
        for block in expected:
            issues = {}
            selected = matrix.select_attempt(series, config, block, issues)
            if selected is None:
                missing.append(dict(group=block["group"], issues=issues))
                continue
            attempt, receipt = selected
            folder = series / "blocks" / attempt
            persisted = folder / "reconciled.json"
            if block["group"] not in granted_groups:
                missing.append(dict(group=block["group"], issues={attempt: "central timing release pending"}))
                continue
            if not persisted.is_file():
                missing.append(dict(group=block["group"], issues={attempt: "dispatch reducer receipt pending"}))
                continue
            assert run.read_json(persisted) == json.loads(json.dumps(receipt))
            assert len(receipt["raw_sha256"]) == len(block["order"]) == 6
            for name, expected_digest in receipt["raw_sha256"].items():
                raw = folder / Path(name) / "raw.tar.gz"
                assert run.digest(raw) == expected_digest
            matched.append(dict(group=block["group"], attempt=attempt, **receipt))
    else:
        missing = [dict(group=block["group"], issues={}) for block in expected]

    return dict(setup_release=release["released_at"], timing_releases=timing_releases,
                matched=matched, pending=missing,
                qualified_timing_sha256=run.digest(qualification))


if __name__ == "__main__":
    result = verify()
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    raise SystemExit(bool(result["pending"]))
