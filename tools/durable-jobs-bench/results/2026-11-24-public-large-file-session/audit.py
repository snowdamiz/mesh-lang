"""Decode released or newly completed public LARGE raw receipts, without a host call."""

import hashlib
import json
import random
from statistics import median
import sys
import tarfile

from build import ELF, HERE, ROOT, sha

SOURCE = ROOT / "target/2026-11-24-trusted-jobs-product-composition/source"
PUBLIC = "2026-11-24-trusted-jobs-product-composition"
RESULTS = HERE.parent
PUBLIC_REPLAY = SOURCE / HERE.parent.relative_to(ROOT) / PUBLIC / "replay"
sys.path.insert(0, str(PUBLIC_REPLAY))
import intake

BASELINE = RESULTS / PUBLIC / "evidence/native/timing-closure-retry"
COMPARATORS = ("river", "oban", "apalis")
FIELDS = ("attempt_xid", "effect_xid", "settled_xid")
FIXTURE = "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1"


def historical():
    observations = {}
    for series, first, second in (
        ("direct-producer-v3-large", "mesh-stock-a", "mesh-direct-file"),
        ("payload-storage-mesh", "mesh-external", "mesh-direct-external"),
        ("payload-storage-symmetric", "mesh-external", "mesh-direct-external"),
        ("ordinary-trusted-v1-large/evidence/ordinary-trusted-large-assembly-matched",
         "mesh-direct-external", None),
    ):
        rows = []
        folder = RESULTS / ("2026-11-23-" + series) / "blocks"
        for ref in sorted(folder.glob("*/reconciled.json")):
            rpt = intake.read(ref)
            scores = rpt["scores"]
            selected = {key: {name: value for name, value in score.items()
                              if name in ("first_submit_to_commit_ms", "submit_to_settlement_p99_ms",
                                          "submit_jobs_per_s", "commit_jobs_per_s", "raw_sha256")}
                        for key, score in scores.items() if key.startswith("mesh") or key == "ordinary"}
            savings = scores[first]["first_submit_to_commit_ms"] - scores[second]["first_submit_to_commit_ms"] \
                if first in scores and second in scores else None
            if savings is not None and "mesh-stock-b" in scores:
                savings = min(savings, scores["mesh-stock-b"]["first_submit_to_commit_ms"]
                              - scores[second]["first_submit_to_commit_ms"])
            rows.append(dict(receipt=str(ref.relative_to(ROOT)), receipt_sha256=sha(ref),
                             conservative_producer_saving_ms=savings, scores=selected))
        observations[series] = rows
    once = RESULTS / "2026-11-23-ordinary-broker-once/evidence/native-summary.json"
    observations["broker-once"] = dict(receipt=str(once.relative_to(ROOT)), sha256=sha(once),
                                       same_transport="both direct-file; ordinary V1")
    return observations


def decode_block(qualified, block, expected_hashes):
    group = intake.read(block / "order.json")["group"]
    scores = intake.compare(qualified, block, group)
    recipe = qualified["recipe"]
    result = {}
    for phase in ("warmup", "measured"):
        for label, lane in qualified["configuration"]["lanes"].items():
            engine = lane["engine"]
            path = block / f"{phase}-{label}" / f"{phase}-{engine}/raw.tar.gz"
            with tarfile.open(path) as t:
                def get(name):
                    return intake.run.file_json(t, f"adapter/{name}.json")

                ledger = get("ledger")
                rows = ledger["jobs"]
                assert len(ledger["transactions"]) == 1
                assert ledger["input_sha256"] == FIXTURE and len(rows) == 64
                assert expected_hashes == {row["job_key"]: row["payload_sha256"] for row in rows}
                assert all(row["state"] == "completed" and row["attempt"] == row["attempts"] == row["effects"] == 1
                           for row in rows)
                assert all(len({row[k] for k in FIELDS}) == 3 for row in rows)
                terminal = len({row["settled_xid"] for row in rows})
                app_xids = {row[k] for row in rows for k in FIELDS[:2]}
                assert len(app_xids) == 128
                storage_record = None
                if engine == "mesh":
                    assert terminal == 64 and len({row[k] for row in rows for k in FIELDS}) == 192
                    writer = get("producer-writer")
                    assert (writer["enabled"], writer["bulk"], writer["scalar"]) == (True, 0, 64)
                    tls = get("postgres-tls-sessions")
                    assert sum(row["role"] == "parity_producer" and row["tls"] for row in tls) == 1
                    identities = get("process-identity")
                    session = [row for row in identities if row["argv"][-1] == "session"]
                    assert len(session) == 1
                    assert all(row["executable_sha256"] == ELF for row in identities)
                    assert get("database")["durable"] is True
                    assert t.extractfile("adapter/broker-output.log").read().splitlines()[-1] == b"{}"
                    storage_record = get("storage-topology")
                    assert (storage_record["storage"], storage_record["work_mem"]) == (recipe["storage"], "4MB")
                elif recipe["storage"] != "default":
                    payload = get("payload-storage")
                    persisted = payload["persisted"]
                    stored = persisted["rows"]
                    assert payload["policy"] == "EXTERNAL" and persisted["metadata"]["attstorage"] == "e"
                    assert len(stored) == 64 and all(row["compression"] is None and row["payload_bytes"] == 524288
                                                     for row in stored)
                    assert expected_hashes == {row["job_key"]: row["payload_sha256"] for row in stored}
                    storage_record = dict(policy=payload["policy"], metadata=persisted["metadata"],
                                          toast_heap_bytes=persisted["toast_heap_bytes"],
                                          stored_column_bytes=sum(row["stored_column_bytes"] for row in stored))
                if phase == "measured":
                    metric = scores[label]
                    submit_ms = (metric["last_settlement_us"] - metric["first_submit_us"]) / 1000
                    drain_ms = (metric["last_settlement_us"] - metric["first_commit_us"]) / 1000
                    result[label] = dict(submit_ms=submit_ms, drain_ms=drain_ms,
                                         enqueue_ms=metric["first_submit_to_commit_ms"],
                                         jobs_s=metric["submit_jobs_per_s"], drain_jobs_s=metric["commit_jobs_per_s"],
                                         xids=dict(application=len(app_xids), terminal=terminal),
                                         raw_sha256=metric["raw_sha256"], storage=storage_record)
    product = result["product"]
    paired = {peer: {clock: round(product[clock] - result[peer][clock], 3)
                     for clock in ("submit_ms", "enqueue_ms", "drain_ms")} for peer in COMPARATORS}
    return dict(group=group, block=block.name, arms=result, product_minus_peer_ms=paired)


def audit():
    hashes = {}
    sample = RESULTS / "2026-11-24-enqueue-spec-preparation/large-capture.json"
    # The payload hashes come from the exact seeded fixture independently of the
    # selected public run. Derive them from the sealed profile configuration.
    profiles = SOURCE / "scripts/jobs-parity-profiles.json"
    settings = intake.read(profiles)
    workload = settings["workloads"]["large"]
    rng = random.Random(settings["seed"])
    payloads = {}
    for index in range(64):
        data = rng.randbytes(workload["payload_bytes"] // 2).hex().encode()
        key = f"job_{index:06d}"
        payloads[key] = data.decode()
        hashes[key] = hashlib.sha256(data).hexdigest()
    batch = dict(request_id="batch_000000", rollback=False, jobs=[
        dict(key=key, payload=payload, hold_ms=workload["hold_ms"], fail_first=False)
        for key, payload in payloads.items()
    ])
    original = (json.dumps(batch, indent=2, sort_keys=True) + "\n").encode()
    fixture_file_sha = hashlib.sha256(original).hexdigest()
    assert hashlib.sha256(fixture_file_sha.encode()).hexdigest() == FIXTURE
    inline_bytes = len(json.dumps(dict(verb="enqueue", batch=batch), separators=(",", ":")).encode()) + 1
    lanes = {}
    for name in ("large", "large-external"):
        path = BASELINE / f"{name}.inventory.json"
        if not path.exists():
            lanes[name] = dict(blocks=[], pending="inventory")
            continue
        qualified = intake.read(path)
        assert qualified["artifact"]["files"]["mesh"] == ELF
        assert qualified["configuration"]["lanes"]["product"]["product_mode"] == "product-bulk"
        results = [decode_block(qualified, ref.parent, hashes)
                   for ref in sorted((BASELINE / name / "blocks").glob("*/reconciled.json"))]
        assert len({row["group"] for row in results}) == len(results)
        lanes[name] = dict(inventory_sha256=sha(path), blocks=results)
        if results:
            lanes[name]["median"] = {
                label: {key: median(block["arms"][label][key] for block in results)
                        for key in ("submit_ms", "enqueue_ms", "drain_ms", "jobs_s", "drain_jobs_s")}
                for label in qualified["configuration"]["lanes"]
            }
    return dict(workload_jobs=64, payload_bytes=64 * 524288, historical=historical(),
                original_fixture_bytes=len(original), original_fixture_sha256=fixture_file_sha,
                inline_request_bytes_including_newline=inline_bytes,
                public=lanes, spec_component=dict(capture_sha256=sha(sample), frames=64,
                                                   frame_bytes=524470, combined_bytes=33566080,
                                                   measured_saving_ms=34.536))


if __name__ == "__main__":
    print(json.dumps(audit(), sort_keys=True, indent=2))
