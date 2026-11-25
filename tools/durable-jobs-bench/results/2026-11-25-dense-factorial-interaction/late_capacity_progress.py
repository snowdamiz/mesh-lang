"""Replay runner occupancy against the late-capacity pilot's measured controls."""
import argparse
import json
from pathlib import Path

from runner_work import capture


def replay(evidence):
    release = json.loads((evidence / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0
    assert release["request_sha256"] == "edaec0dfd35a0ebc1bf0df59adc0c8ff436c183163280a8e9d6630231c3f48f4"
    scored = json.loads((evidence / "scored-before-release.json").read_bytes())
    groups = {}
    for group in ("1", "2"):
        assert scored["gates"][group]["passed"]
        groups[group] = {}
        for lane in ("A1", "A2", "reference", "B"):
            cell = "measured-" + lane
            score = scored["groups"][group][cell]
            arm = evidence / "blocks" / f"{int(group):02}-noop" / cell / "measured-mesh"
            groups[group][lane] = capture((arm / "raw.tar.gz").read_bytes(), score["raw_sha256"], 8)
    return groups


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    print(json.dumps(replay(parser.parse_args().evidence), indent=2, sort_keys=True))
