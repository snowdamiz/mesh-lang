"""Read native encumbered claim timings and their uninstrumented duplicates."""
import hashlib
import json
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "2026-11-24-public-dense-set-claim/evidence/native"


def read_member(archive, name):
    return json.load(archive.extractfile(name))


def replay():
    release = json.loads((ROOT / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0 and release["groups"] == [1, 2]
    scored = json.loads((ROOT / "final-replay.json").read_bytes())
    assert scored["failed"] == {}
    results = []
    for order, (block, group) in sorted(scored["groups"].items()):
        for config in ("equal", "stock"):
            hashes = {}
            for variant in ("off-a", "trace", "off-b"):
                name = f"{config}-{variant}"
                score = group["arms"][name]
                raw = ROOT / "blocks" / block / f"measured-{name}/measured-mesh/raw.tar.gz"
                digest = hashlib.sha256(raw.read_bytes()).hexdigest()
                assert digest == score["raw_sha256"]
                hashes[variant] = digest
                with tarfile.open(raw) as archive:
                    assert read_member(archive, "status.json")["exit_code"] == 0
                    ledger = read_member(archive, "adapter/ledger.json")
                    jobs = ledger["jobs"]
                    assert len(jobs) == len({j["job_key"] for j in jobs}) == 1024
                    assert all(j["state"] == "completed" and j["effects"] == j["attempts"] == 1 for j in jobs)
                    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
                    if variant == "trace":
                        stats = read_member(archive, "adapter/claim-off-clock.json")
                        assert score["functions_off_clock"] == stats["functions"]
                        assert "track_functions=pl" in stats["role_settings"]
            fence, = [f for f in stats["functions"] if f["funcname"] == "fence_claim_v1"]
            row_pause, = [s for s in stats["statements"] if "namespace = NEW.namespace" in s["query"]]
            batch_pause, = [s for s in stats["statements"] if "name =" in s["query"] and
                            "EXISTS (SELECT FROM mesh_jobs.queues WHERE namespace = ns" in s["query"]]
            assert fence["calls"] == row_pause["calls"] == 1024
            assert not row_pause["toplevel"] and not batch_pause["toplevel"]
            results.append(dict(order=int(order), lane=config, raw_sha256=hashes,
                                fence_calls=fence["calls"], fence_total_ms=fence["total_time"],
                                row_pause_lookup_calls=row_pause["calls"],
                                row_pause_lookup_exec_ms=row_pause["total_exec_time"],
                                batch_pause_lookup_calls=batch_pause["calls"],
                                batch_pause_lookup_exec_ms=batch_pause["total_exec_time"],
                                perturbation=group["perturbation"][config]))
    return results


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
