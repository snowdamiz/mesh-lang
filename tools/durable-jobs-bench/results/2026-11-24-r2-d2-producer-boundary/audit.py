"""Replay the five published short Mesh arms and their paired peer clocks."""
from hashlib import file_digest
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
SERIES = HERE.parent / ("2026-11-24-exit-confirmed-bulk-d2/evidence/native-timing/"
                        "2026-11-24-exit-confirmed-bulk-d2")
INPUT = "ac0bcbf2465ea6d6807e8bfcdcfc3b1dacfa18b775e3f4c074257d66bfbba9d1"
BROKER = "55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a"
RUNNER = "972514efac688fd029821152c3bb70a7e62bf1c6227ab8b2d061dc69c47a2dc7"
LANES = {"r2-d2": "mesh", "river": "go", "apalis": "rust", "oban": "oban"}


def digest(path):
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def times(ledger):
    assert ledger["input_sha256"] == INPUT and len(ledger["jobs"]) == 128
    assert ledger["profile"] == "short" and not ledger.get("local_smoke", False)
    batch, = ledger["transactions"]
    assert len(batch["enqueued_ids"]) == 128
    assert len(set(batch["enqueued_ids"].values())) == 128
    assert all(job["state"] == "completed" for job in ledger["jobs"])
    ack = batch.get("acknowledged_us", batch.get("returned_us"))
    assert batch["submitted_us"] <= batch["committed_us"] <= ack
    return dict(submit_us=batch["submitted_us"], commit_us=batch["committed_us"],
                ack_us=ack, ack_field="acknowledged_us" if "acknowledged_us" in batch else "returned_us",
                last_terminal_us=max(job["settled_us"] for job in ledger["jobs"]),
                xid=batch["xid"])


def audit():
    expected = {}
    for line in (SERIES / "SHA256SUMS").read_text().splitlines():
        sha, target = line.split(maxsplit=1)
        expected[target.removeprefix("./")] = sha
    result = []
    for block in sorted((SERIES / "blocks").iterdir()):
        order = json.loads((block / "order.json").read_text())
        if order["profile"] != "short" or order["phases"] != ["measured"]:
            raise ValueError(f"unexpected short block: {block}")
        lanes = {}
        for lane, engine in LANES.items():
            archive = block / f"measured-{lane}/measured-{engine}/raw.tar.gz"
            sha = digest(archive)
            rel = str(archive.relative_to(SERIES))
            assert expected[rel] == sha
            with tarfile.open(archive) as raw:
                def read(name):
                    with raw.extractfile(f"adapter/{name}.json") as stream:
                        return json.load(stream)
                ledger = read("ledger")
                row = times(ledger)
                row.update(archive=rel, sha256=sha,
                           submit_to_commit_us=row["commit_us"] - row["submit_us"],
                           submit_to_terminal_us=row["last_terminal_us"] - row["submit_us"],
                           commit_to_ack_us=row["ack_us"] - row["commit_us"])
                if lane == "r2-d2":
                    witness = read("producer-writer")
                    assert witness["enabled"] and witness["bulk"] == 1 and witness["scalar"] == 0
                    split = read("split-executables")
                    assert (split["broker_sha256"], split["runner_sha256"]) == (BROKER, RUNNER)
                    stmts = read("trusted-application-statements")
                    for table in ("attempts", "effects"):
                        stmt, = [s for s in stmts if s["query"].startswith(
                            f"INSERT INTO public.mesh_jobs_parity_{table}")]
                        assert stmt["rows"] == stmt["calls"] == 128
                    row["writer"] = witness
                lanes[lane] = row
        result.append(dict(group=order["group"], scores=lanes, gap_us={
            lane: lanes["r2-d2"]["submit_to_commit_us"] - lanes[lane]["submit_to_commit_us"]
            for lane in ("river", "apalis")
        }))
    assert [row["group"] for row in result] == list(range(1, 6))
    assert all(min(row["gap_us"].values()) > 0 for row in result)
    return result


if __name__ == "__main__":
    print(json.dumps(audit(), sort_keys=True, indent=2))
