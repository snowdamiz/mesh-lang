"""Read the pre-existing matched Linux crossover. This script has no dispatch."""
import hashlib
import json
from pathlib import Path
import sys
import tarfile

ENGINE = {"bulk": "mesh", "river": "go", "oban": "oban", "apalis": "rust"}


def load(path):
    return json.loads(path.read_text())


def matched(series):
    summary = load(series / "summary.json")
    config = load(series / "series.json")["config"]
    assert summary["cpus"] == config["cpus"] == [0, 1] and not summary["missing"]
    peers = {}
    for label, engine in ENGINE.items():
        pinned = config["lanes"][label]
        assert pinned["engine"] == engine and pinned["inventory"]["cpus"] == [0, 1]
        peer = pinned["inventory"]["engines"][engine]
        peers[label] = {key: peer[key] for key in ("version", "artifact_sha256", "verified_files")}
    boot, kernel, mount = summary["host"]
    blocks = summary["rows"]["short"]
    assert len(blocks) == 5
    reference = []
    for block in blocks:
        rows = {}
        for label, engine in ENGINE.items():
            score = block["scores"][label]
            assert score["host_boot_id"] == boot and score["host_kernel"] == kernel
            assert score["pg_mount"]["maj:min"] == mount and score["cpu_steal_percent"] <= 1
            assert score["completed"] == score["ordered_independent_writes"] == 128
            relative = f"measured-{label}/measured-{engine}"
            archive = series / "blocks" / block["attempt"] / relative / "raw.tar.gz"
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            assert digest == block["raw_sha256"][relative]
            with tarfile.open(archive) as source:
                ledger = json.load(source.extractfile("adapter/ledger.json"))
            assert len(ledger["jobs"]) == 128
            entries = sorted(r["entered_us"] for r in ledger["jobs"])
            result = {key: score[key] for key in ("submit_jobs_per_s", "commit_jobs_per_s",
                "runner_cpu_ms_per_job", "first_submit_to_commit_ms")}
            result.update(raw_sha256=digest, submit_ms=(score["last_settlement_us"]-score["first_submit_us"])/1000,
                          drain_ms=(score["last_settlement_us"]-score["first_commit_us"])/1000,
                          second_half_entry_ms=(entries[-1]-entries[63])/1000,
                          entry_ms=[(entries[rank-1]-score["first_commit_us"])/1000 for rank in (1,64,128)])
            rows[label] = result
        reference.append(dict(group=block["group"], attempt=block["attempt"], arms=rows))
    return dict(source_commit="22578fbaefb596c3ab4b32f6f17da50bd8d24819",
                series=series.name, boot=boot, kernel=kernel, mount=mount, cpus=summary["cpus"],
                peers=peers, blocks=reference)


if __name__ == "__main__":
    json.dump(matched(Path(sys.argv[1])), sys.stdout, indent=2, sort_keys=True)
    print()
