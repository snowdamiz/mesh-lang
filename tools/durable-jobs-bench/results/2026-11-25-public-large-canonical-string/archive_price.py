"""Reconcile the real FILE frames and the frozen stock admission digest."""
import hashlib
import json
import statistics
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OLD = ROOT / "2026-11-24-enqueue-spec-preparation"
PUBLIC = ROOT / "2026-11-24-public-large-file-session/evidence/trace-file-replay.json"
FROZEN = OLD / "evidence/timing-be34d253e19e/raw"
STOCK_DIGEST = "c98a0cd9500d920dfb07b040ffc7a90eb2628f82681eed196f2ea987db376e18"


def read(path):
    return json.loads(path.read_bytes())


def main():
    pins = read(PUBLIC)["frames"]
    assert pins == read(OLD / "large-capture.json")["frames"]
    assert len(pins) == 64
    fingerprint_chain = hashlib.sha256()
    total_input = total_payload = eligible = 0
    with tarfile.open(OLD / "native-amd64-source.tar.gz") as tar:
        for i, pin in enumerate(pins):
            frame = tar.extractfile(f"frames/large/{i}").read()
            assert hashlib.sha256(frame).hexdigest() == pin["sha256"]
            assert len(frame) == pin["bytes"]
            spec, = json.loads(frame)
            key = spec.pop("request_key")
            args = spec["args"]
            payload = args["payload"].encode()
            assert len(payload) == 524288 and len(payload) >= len(frame)//2
            assert not any(c < 32 or c in (34,92) for c in set(payload))
            eligible += 1
            total_input += len(frame)
            total_payload += len(payload)
            canonical = dict(version=1, queue="default", metadata={}, priority=0, tags=[],
                             scheduled_at_ms=None, delay_ms=None, timeout_ms=60000,
                             suspended=False, unique=None, replace={}, **spec)
            wire = json.dumps(canonical, sort_keys=True, separators=(",", ":")).encode()
            fingerprint_chain.update(b"\x01" + (1).to_bytes(8, "big"))
            for field in (wire, key.encode(), hashlib.sha256(wire).hexdigest().encode()):
                fingerprint_chain.update(len(field).to_bytes(8, "big"))
                fingerprint_chain.update(field)
    assert fingerprint_chain.hexdigest() == read(FROZEN / "large-semantic.json")["output_sha256"] == STOCK_DIGEST
    summary = read(FROZEN / "summary.json")
    savings = summary["profiles"]["large"]["cpu_saving_vs_faster_stock_ms"]
    assert len(savings) == 6
    peer_path = ROOT / "2026-11-25-public-large-file-external/grant-parser-retry/evidence/replay.json"
    peer_bytes = peer_path.read_bytes()
    assert hashlib.sha256(peer_bytes).hexdigest() == "e633ff4fcafddf0f05c66affdcc931aa5bef4c829e64b38f15c8d1d2d07694a1"
    budgets = []
    for group in json.loads(peer_bytes)["groups"]:
        clocks = group["clocks"]
        current = clocks["file"]
        submit_savings = {
            peer: round(max(0, current["submit_ms"] - clocks[peer]["submit_ms"] / 1.2), 3)
            for peer in ("river", "oban", "apalis")
        }
        budgets.append(dict(group=group["group"], extra_ms_for_1_2x_submit=submit_savings,
                            ms_to_tie_oban_drain=round(current["drain_ms"] - clocks["oban"]["drain_ms"], 3)))
    print(json.dumps(dict(fixed_drain_sensitivity=budgets, frames=eligible, input_bytes=total_input,
                          eligible_string_bytes=total_payload, stock_output_sha256=STOCK_DIGEST,
                          predecessor_saving_ms=savings,
                          extra_saving_for_40ms_median=40-statistics.median(savings)),
                     indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
