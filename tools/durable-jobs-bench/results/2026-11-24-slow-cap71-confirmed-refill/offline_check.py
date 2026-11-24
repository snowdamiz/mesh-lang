"""Check the plan and archived cap-71 peers without launching any executable."""
import json
from pathlib import Path
import sys
import tarfile

from controller import CAP_EVIDENCE, FUSED, HERE, PEERS, PUBLIC, PUBLIC_SETUP, peer_inventory, plan, setup_artifact, short_adapter_receipt
from score import FUSED as FUSED_LANES, GROUP, METRICS, gate, run, score_arm


def compare_sources():
    file_prefix = "source/tools/durable-jobs-bench/results"
    files = ("main.mpl", "writes.mpl", "worker_identity.c", "worker_identity.mpl")
    images = {}
    for name in (PUBLIC, FUSED):
        with tarfile.open(HERE.parent / name / "source.tar.gz") as archive:
            images[name] = {f: archive.extractfile(f"{file_prefix}/{name}/fixture/{f}").read() for f in files}
    for file in files:
        first, second = (images[image][file] for image in (PUBLIC, FUSED))
        if file == "main.mpl":
            delimiter = b"fn serve_started("
            assert delimiter in first and delimiter in second
            first, second = (text.split(delimiter, 1)[0] for text in (first, second))
        assert first == second, file


def check_gate():
    measured = {name: dict.fromkeys((*METRICS, "outside_slot_ms"), 100) for name in (*GROUP, "public-group")}
    for name in FUSED_LANES:
        measured[name] = dict.fromkeys((*METRICS, "outside_slot_ms"), 90)
        measured[name]["refill"] = [{"grants": 50}, {"grants": 50}]
    assert gate(measured)["advance"]
    measured["fused-b"]["submit_ms"] = 95
    assert not gate(measured)["advance"]
    measured["fused-b"]["submit_ms"] = 94
    measured["fused-a"]["submit_ms"] = 88
    assert not gate(measured)["advance"]


def check_peers():
    design = run.read_json(CAP_EVIDENCE / "series.json")["design"]
    blocks = CAP_EVIDENCE.parent / "blocks"
    actual = sorted(blocks.glob("01-slow.*"))[0]
    peers = {}
    for label, engine in PEERS.items():
        old = run.read_json(CAP_EVIDENCE / f"staging/{engine}/inventory.json")
        current = peer_inventory(label, engine)
        before, after = (inv["engines"][engine] for inv in (old, current))
        assert before["artifact_sha256"] == after["artifact_sha256"]
        assert sorted(before["verified_files"].values()) == sorted(after["verified_files"].values())
        assert Path(after["argv"][1]).parent.name.endswith(label)
        binding = {"design": design, "lanes": {label: {"engine": engine, "inventory": old}}}
        folder = actual / f"measured-{engine}" / f"measured-{engine}"
        row = score_arm(folder, label, "measured", binding)
        peers[label] = dict(artifact=after["artifact_sha256"], replayed_raw_sha256=row["raw_sha256"],
                            jobs=row["completed"], hold_peak=row["committed_hold_peak"], write_xids=row["write_xids"])
    return peers


def main():
    compare_sources()
    first, second, *_ = plan()
    assert first["order"] == second["order"][::-1] and len(set(first["order"])) == 8
    check_gate()
    _, public = setup_artifact(PUBLIC_SETUP, "public")
    public["short_adapter_qualification"] = short_adapter_receipt(PUBLIC_SETUP, "public", public)
    data = dict(source_sections_match=True, reversed_orders=True, duplicate_gate=True, public_native_setup=public,
                archived_peer_replay=check_peers(), workload_launched=False)
    json.dump(data, sys.stdout, indent=2, sort_keys=True)
    print()


if __name__ == "__main__":
    main()
