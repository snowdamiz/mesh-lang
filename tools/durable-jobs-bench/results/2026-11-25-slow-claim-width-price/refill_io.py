"""Read device flush and host I/O pressure counters strictly inside cap68 drain."""
from hashlib import sha256
import json
from pathlib import Path
import sys
import tarfile


def counters(frame, device):
    fields, = [line.split()[3:] for line in frame["diskstats"]
               if ":".join(line.split()[:2]) == device]
    pressure = {line.split()[0]: int(line.split()[-1].removeprefix("total="))
                for line in frame["io_pressure"].splitlines()}
    return {"device_busy_ms": int(fields[9]), "device_flushes": int(fields[15]),
            "device_flush_ms": int(fields[16]), "host_io_some_us": pressure["some"],
            "host_io_full_us": pressure["full"]}


def summarize(raw_path, digest):
    assert sha256(raw_path.read_bytes()).hexdigest() == digest
    with tarfile.open(raw_path) as raw:
        assert json.load(raw.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(raw.extractfile("adapter/ledger.json"))
        frames = list(map(json.loads, raw.extractfile("host-samples.jsonl")))
        mount = json.load(raw.extractfile("host.json"))["pg_mount"]
    txn, = ledger["transactions"]
    assert len(ledger["jobs"]) == len({j["settled_xid"] for j in ledger["jobs"]}) == 512
    last = max(j["settled_us"] for j in ledger["jobs"])
    pairs = list(zip(frames, frames[1:]))
    assert all(a["epoch_us"] < b["epoch_us"] and
               abs(b["epoch_us"] - a["epoch_us"] - (b["monotonic_ns"] - a["monotonic_ns"]) / 1000) < 1000
               for a, b in pairs)
    # Diskstats and PSI follow the epoch stamp, before the following frame.
    inside = [(a, b) for a, b in pairs if txn["committed_us"] <= a["epoch_us"] and b["epoch_us"] <= last]
    result = {"raw_sha256": digest, "drain_us": last - txn["committed_us"], "interior_reads": len(inside)}
    if len(inside) < 2:
        return result
    start, after_start = inside[0]
    end, after_end = inside[-1]
    low, high = counters(start, mount["maj:min"]), counters(end, mount["maj:min"])
    delta = {key: high[key] - low[key] for key in high}
    assert min(delta.values()) >= 0
    return dict(result, **delta, read_span_min_us=end["epoch_us"] - after_start["epoch_us"],
                read_span_max_us=after_end["epoch_us"] - start["epoch_us"])


def replay(directory):
    release = (directory / "controller-release.json").read_bytes()
    assert sha256(release).hexdigest() == "1bd34ec7973f01b2d57c7ba93e27741e03906f088d1d9962f99eedf09cdebde1"
    archives = json.loads(release)["raw_archives"]
    assert len(archives) == 20
    return {name: summarize(directory / name, digest) for name, digest in sorted(archives.items())}


if __name__ == "__main__":
    print(json.dumps(replay(Path(sys.argv[1])), indent=2, sort_keys=True))
