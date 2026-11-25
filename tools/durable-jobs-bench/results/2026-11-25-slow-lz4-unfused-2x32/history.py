"""Read the three separate released comparisons and rederive the SLOW phase intervals."""
import hashlib
import json
from pathlib import Path
from statistics import median
import tarfile

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
CAP = RESULTS / "2026-11-24-ordinary-slow-equal-topology-cap71"
EQUAL = CAP / "evidence/native-analysis/timing-replay.json"
MIXED = (RESULTS / "2026-11-23-ordinary-slow-runner-concentration/evidence/native-import-retry"
         / "native-followup-replay.json")
CODEC = (RESULTS / "2026-11-25-slow-lz4-isolated-qualification/evidence/native/recovery-3"
         / "timing-continuation/independent-replay.json")


def read(path):
    return json.loads(path.read_text())


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def compare(arms, mesh, peers, fields=("submit_ms", "drain_ms")):
    return {name: [arms[mesh][key] - arms[peer][key] for key in fields] for name, peer in peers.items()}


def raw_phases(block, engine, expected):
    path = CAP / "evidence/native/sql-retry-timing/blocks" / block / f"measured-{engine}/measured-{engine}/raw.tar.gz"
    digest_ = digest(path)
    if digest_ != expected:
        raise ValueError(f"archived raw SHA changed: {path}")
    with tarfile.open(path) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        jobs = ledger["jobs"]
        txn, = ledger["transactions"]
        if len(jobs) != 512 or any(job["hold_ms"] != 100 for job in jobs):
            raise ValueError("wrong released slow input")
        first64 = sorted(job["attempt_committed_us"] for job in jobs)[63]
        last_effect = max(job["effect_committed_us"] for job in jobs)
        terminal = max(job["settled_us"] for job in jobs)
        row = dict(raw_sha256=digest_, phases_ms=[(first64-txn["committed_us"])/1000,
                    (last_effect-first64)/1000, (terminal-last_effect)/1000])
        if engine == "go":
            samples = [json.loads(line) for line in archive.extractfile("adapter/river/measured/samples.jsonl")]
            row.update(idle_tls_clients=[sample["connections"] for sample in samples
                                        if sample["idle"] and sample["all_connections_tls"]],
                       in_load_peak=max(sample["connections"] for sample in samples if not sample["idle"]))
    return row


def report():
    original = read(EQUAL)
    equal = {}
    for block, arms in original["qualified"].items():
        pair = {engine: raw_phases(block, engine, arms[f"measured-{engine}"]["raw_sha256"])
                for engine in ("mesh", "go")}
        equal[block] = dict(mesh_minus_peer_ms=compare(arms, "measured-mesh",
                            {"river":"measured-go", "oban":"measured-oban", "apalis":"measured-rust"}),
                            mesh_river_phase_inputs=pair,
                            river_minus_mesh_phases_ms=[a-b for a,b in zip(pair["go"]["phases_ms"],
                                                                           pair["mesh"]["phases_ms"], strict=True)])
    mixed = {group: compare(arms, "measured-two", {name: f"measured-{name}-stock-1x64"
                                                  for name in ("river", "oban", "apalis")})
             for group, arms in read(MIXED)["attempts"].items()}
    codec = read(CODEC)
    return {
        "clock_order": ["submit_to_terminal", "commit_to_terminal"],
        "phase_order": ["commit_to_64th_first", "64th_first_to_last_effect", "last_effect_to_terminal"],
        "source_sha256": {path.relative_to(RESULTS).as_posix(): digest(path) for path in (EQUAL, MIXED, CODEC)},
        "equal_cap71_stock": equal,
        "mixed_cap68_stock_mesh_minus_peer_ms": mixed,
        "same_image_one_by_64_codec": {
            "mesh_minus_peer_ms": codec["within_block_mesh_minus_peer_ms"],
            "pglz_minus_lz4_ms": codec["within_block_pglz_minus_lz4"],
        },
        "equal_cap71_phase_summary_ms_river_minus_mesh": [
            {"median": median(values), "min": min(values), "max": max(values)}
            for values in zip(*(row["river_minus_mesh_phases_ms"] for row in equal.values()), strict=True)
        ],
    }


if __name__ == "__main__":
    print(json.dumps(report(), sort_keys=True, indent=2))
