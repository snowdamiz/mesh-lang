"""Measure the normalized arguments of the dense producer's eight bulk calls."""
import gzip
from hashlib import sha256
import json

from file_transport_price import DENSE, RESULTS, measured


def encode(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()


def rows():
    path = RESULTS / "2026-11-25-dense-enqueue-jsonb-path/evidence/stock-raw.json.gz"
    compressed = path.read_bytes()
    assert sha256(compressed).hexdigest() == "3a529840f260ffa029d3c4d1fc0c7099a634bf1925e96545adc786f6e3751a52"
    run = json.loads(gzip.decompress(compressed))["compiled"]["dense"]
    assert run["calls"] == {"bulk": [8, 1024]}
    durable = run["durable"]
    receipts = {r["request_key"]: r for r in durable["receipts"]}
    assert len(receipts) == len(durable["jobs"]) == 1024
    stored_fields = ("worker", "version", "queue", "args", "metadata", "priority", "tags",
                     "max_attempts", "timeout_ms", "backoff_base_ms", "backoff_max_ms")
    result = []
    for position, job in enumerate(durable["jobs"], 1):
        assert job["position"] == position
        key = job["args"]["key"]
        receipt = receipts[key]
        assert receipt["receipt"]["id"] == run["broker"]["ids"][key] == job["id"]
        spec = {field: job[field] for field in stored_fields}
        spec.update(scheduled_at_ms=None, delay_ms=None, suspended=False, unique=None, replace={})
        fingerprint = sha256(encode(spec)).hexdigest()
        assert fingerprint == receipt["fingerprint"]
        result.append(dict(spec=spec, request_key=key, fingerprint=fingerprint))
    return result


def replay():
    admissions = rows()
    batch_sizes = []
    for start in range(0, len(admissions), 128):
        group = admissions[start:start + 128]
        common = {k: v for k, v in group[0]["spec"].items()
                  if all(row["spec"][k] == v for row in group)}
        assert set(group[0]["spec"]) - common.keys() == {"args"}
        individual = [dict(args=row["spec"]["args"], request_key=row["request_key"],
                           fingerprint=row["fingerprint"]) for row in group]
        for item, original in zip(individual, group):
            assert {**common, "args": item["args"]} == original["spec"]
        batch_sizes.append(dict(stock_json_bytes=len(encode(group)),
                                shared_json_bytes=len(encode(dict(common=common, rows=individual))),
                                shared_fields=len(common)))

    payloads = {row["request_key"]: sha256(row["spec"]["args"]["payload"].encode()).hexdigest()
                for row in admissions}
    scores = json.loads((DENSE / "scored-before-release.json").read_bytes())["groups"]
    assert json.loads((DENSE / "controller-release.json").read_bytes())["exit_status"] == 0
    pairs = []
    for i in range(1, 6):
        block = DENSE / "blocks" / f"{i:02}-noop"
        pair = {name: measured(block, name, scores[str(i)]["measured-" + name]["raw_sha256"], payloads)
                for name in ("B", "river")}
        pairs.append(pair)
    return dict(batch_sizes=batch_sizes, verified_fingerprints=len(admissions), native_pairs=pairs)


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
