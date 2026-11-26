"""Price the two ways to avoid encoding an overflow spec twice in the FILE broker."""
import argparse
import hashlib
import json
from pathlib import Path
import tarfile


def reduce_visits(evidence, canonical):
    release_bytes = canonical.read_bytes()
    assert release_bytes == (evidence / "controller-release.json").read_bytes()
    release = json.loads(release_bytes)
    assert release["exit_status"] == 0 and release["id"].startswith("public-file-json-6c1f5eef6e19-")
    custody_bytes = (evidence / "custody.json").read_bytes()
    assert hashlib.sha256(custody_bytes).hexdigest() == release["custody_sha256"]
    custody = json.loads(custody_bytes)
    proof = (evidence / "independent-replay.json").read_bytes()
    assert hashlib.sha256(proof).hexdigest() == custody["replay-before-release.json"]
    replay = json.loads(proof)
    status_bytes = (evidence / "native-status.json").read_bytes()
    assert hashlib.sha256(status_bytes).hexdigest() == custody["native-status.json"]
    status = json.loads(status_bytes)
    assert replay["complete"] and status["native_exit"] == status["export_exit"] == 0
    raw = evidence / "raw.tar.gz"
    with raw.open("rb") as f:
        assert hashlib.file_digest(f, "sha256").hexdigest() == custody["raw.tar.gz"] == replay["raw_sha256"] == status["sha256"]
    with tarfile.open(raw) as archive:
        for name, arm in replay["arms"].items():
            if arm["visit_clocks"] is None:
                continue
            samples = []
            for line in archive.extractfile(f"{name}/encode-trace.txt"):
                tag, leaves, size, dirty, encoded, spans, wall, cpu = line.split()
                assert tag == b"public-json-long-text"
                assert (int(leaves), int(size), int(dirty), int(encoded)) == (1, 524288, 0, 524468)
                assert int(spans) == name.endswith("-audit-fast")
                assert 0 < int(cpu) <= int(wall) + 4000
                samples.append((int(cpu), int(wall)))
            assert len(samples) == 127
            sums = tuple(map(sum, zip(*samples)))
            total = arm["visit_clocks"]["all"]
            assert sums == (total["cpu_ns"], total["wall_ns"])
            # Visits 0, 2, ... accept jobs 0..63. Visits 1, 3, ... try jobs 1..63.
            # Carrying the peek saves the later accepted encode. Skipping it saves the peek.
            for label, subset in (("accepted", samples[::2]), ("overflow", samples[1::2]),
                                  ("accepted_again", samples[2::2])):
                cpu, wall = map(sum, zip(*subset))
                print(f"{name} {label} {len(subset)} {cpu / 1e6:.3f} {wall / 1e6:.3f}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("canonical_release", type=Path)
    options = parser.parse_args()
    reduce_visits(options.evidence, options.canonical_release)
