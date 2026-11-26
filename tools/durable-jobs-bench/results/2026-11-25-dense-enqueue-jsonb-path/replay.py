"""Reconcile the archived untimed proof without a database or a broker."""

import gzip
from hashlib import file_digest
import json
import tarfile

from overlay import HERE, STOCK_SHA, body, candidate, digest, source
from prove import normalize

EVIDENCE = HERE / "evidence"
SOURCE = HERE / "source-deb2a3bc49fe.tar.gz"
SOURCE_SHA = "deb2a3bc49feae7df985fe895209c3db7d725cd4c9c1b9c8ba49c93a650dd6d8"
INPUT_SHA = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"


def sha(path):
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(path.read_text())


def verify():
    assert sha(SOURCE) == SOURCE_SHA and sha(HERE / "enqueue-stock.sql") == STOCK_SHA
    assert (HERE / "candidate.sql").read_text() == candidate()
    with tarfile.open(SOURCE) as tar:
        assert tar.extractfile("source/compiler/mesh-rt/src/jobs/enqueue_schema.sql").read() == (
            HERE / "enqueue-stock.sql").read_bytes()
    build, summary = (read(EVIDENCE / name) for name in ("build.json", "qualification.json"))
    assert build["source_sha256"] == SOURCE_SHA and build["files"]["mesh"] == summary["broker_sha256"]
    assert summary["server_version"] == "17.11" and summary["input_sha256"] == INPUT_SHA
    assert (EVIDENCE / "stock-merge-plan.json").read_bytes() == (EVIDENCE / "candidate-merge-plan.json").read_bytes()
    assert "x.value -> 'spec'" in (EVIDENCE / "stock-merge-plan.json").read_text()
    assert "mesh_jobs.now_ms()" in (EVIDENCE / "stock-merge-plan.json").read_text()
    assert "x.value #>> '{spec,queue}'" in (EVIDENCE / "candidate-queue-plan.json").read_text()
    paths = [EVIDENCE / f"{label}-raw.json.gz" for label in ("stock", "candidate")]
    stock, projected = [json.loads(gzip.decompress(path.read_bytes())) for path in paths]
    a, b = (item["state"] for item in (stock, projected))
    assert a["before"] == a["after"] == b["before"]
    assert a["after"]["body"] == digest(body(source()))
    assert b["after"] == dict(a["after"], body=digest(body(candidate())))
    assert stock["direct"] == projected["direct"] and set(stock["direct"]) == set(summary["direct_cases"])
    for variant in (stock, projected):
        compiled = variant["compiled"]
        assert set(compiled) == set(summary["compiled_cases"])
        dense = compiled["dense"]
        assert dense["calls"] == {"bulk": [8, 1024]} and dense["commit_witness"] == [1024, 1024, 1, True]
        assert len(dense["durable"]["jobs"]) == len(dense["durable"]["receipts"]) == 1024
        assert compiled["retry_fresh"]["reply"] == compiled["retry_receipt"]["reply"]
        assert compiled["retry_receipt"]["calls"] == {"bulk": [1, 1], "scalar": [32, 32]}
        assert compiled["legacy"]["calls"] == compiled["acl"]["calls"] == {"scalar": [32, 32]}
        assert compiled["layout"]["calls"] == {"bulk": [1, 1], "scalar": [32, 32]}
        assert compiled["rolled_back"]["durable"] == compiled["savepoint"]["durable"] == dict(
            jobs=[], receipts=[], queues=[], fence=0)
    for label in summary["compiled_cases"]:
        left, right = (item["compiled"][label] for item in (stock, projected))
        assert left["calls"] == right["calls"], label
        assert normalize(left["durable"]) == normalize(right["durable"]), label
        if "fields" in left or label in ("retry_conflict", "savepoint"):
            assert left["reply"] == right["reply"], label
    return dict(summary, raw_sha256={path.name: sha(path) for path in paths})


if __name__ == "__main__":
    print(json.dumps(verify(), indent=2, sort_keys=True))
