"""Measure reversible Zstandard frames for the exact LARGE argument values."""
import argparse
import base64
from hashlib import sha256
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tarfile
from tempfile import TemporaryDirectory

from replay import NATIVE

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT / "scripts"))
PARITY = runpy.run_path(str(ROOT / "scripts/jobs-parity.py"))


def large_input():
    config = json.loads(PARITY["CONFIG"].read_bytes())
    batch, = PARITY["inputs"](config["workloads"]["large"], 64, config["seed"])
    contents = (json.dumps(batch, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
    manifest_hash = sha256(sha256(contents).hexdigest().encode()).hexdigest()
    jobs = batch["jobs"]
    payloads = {j["key"]: sha256(j["payload"].encode()).hexdigest() for j in jobs}
    raw_hashes = []
    for receipt in sorted(NATIVE.glob("blocks/*/reconciled.json")):
        binding = json.loads(receipt.read_bytes())["receipts"]["measured"]["file"]
        raw, = receipt.parent.glob("measured-file/*/raw.tar.gz")
        digest = sha256(raw.read_bytes()).hexdigest()
        assert digest == binding["raw_sha256"]
        with tarfile.open(raw) as archive:
            ledger = json.load(archive.extractfile("adapter/ledger.json"))
            assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
            assert ledger["input_sha256"] == manifest_hash
            assert {j["job_key"]: j["payload_sha256"] for j in ledger["jobs"]} == payloads
        raw_hashes.append(digest)
    assert len(raw_hashes) == 5 and len(jobs) == len(payloads) == 64
    return jobs, manifest_hash, raw_hashes


def argument_bytes(bindir, jobs):
    with TemporaryDirectory(dir=ROOT / "target", prefix="argz-") as tmp:
        base = Path(tmp)
        data = base / "pg"
        ctl = [bindir / "pg_ctl", "-D", data, "-w"]
        subprocess.run([bindir / "initdb", "-D", data, "-A", "trust", "--no-locale", "-E", "UTF8",
                        "-U", "owner"], check=True, stdout=subprocess.DEVNULL)
        subprocess.run([*ctl, "-l", base / "pg.log", "-o", f"-c listen_addresses='' -k {base}",
                        "start"], check=True, stdout=subprocess.DEVNULL)
        try:
            psql = [bindir / "psql", "-XqAt", "-h", base, "-U", "owner", "-d", "postgres", "-v", "ON_ERROR_STOP=1"]
            assert subprocess.check_output(psql, input=b"SHOW server_version_num") == b"170011\n"
            values = []
            for i in range(0, len(jobs), 8):
                group = jobs[i:i + 8]
                encoded = json.dumps(group, allow_nan=False)
                assert "$args$" not in encoded
                sql = ("SELECT arg::text FROM jsonb_array_elements($args$" + encoded
                       + "$args$::jsonb) WITH ORDINALITY AS x(arg, n) ORDER BY n;")
                rows = subprocess.check_output(psql, input=sql.encode()).splitlines()
                assert [json.loads(row) for row in rows] == group
                values.extend(rows)
            return values
        finally:
            subprocess.run([*ctl, "-m", "fast", "stop"], check=True, stdout=subprocess.DEVNULL)


def replay(bindir, zstd):
    jobs, manifest, hashes = large_input()
    values = argument_bytes(bindir, jobs)
    total = sum(map(len, values))
    size = dict(binary=0, base64=0, bytea_hex=0)
    for value in values:
        frame = subprocess.check_output([zstd, "-q", "-1", "-c"], input=value)
        assert subprocess.check_output([zstd, "-q", "-d", "-c"], input=frame) == value
        size["binary"] += len(frame)
        size["base64"] += len(base64.b64encode(frame))
        size["bytea_hex"] += 2 + 2 * len(frame)
    cursors = json.loads((HERE.parent / "2026-11-24-public-large-checked-cursor/receive-intervals.json"
                          ).read_bytes())["external-4m"]
    combined = json.loads((HERE / "factorial-deadlines.json").read_bytes())["combined"]
    scale = []
    for cursor, group in zip(cursors, combined, strict=True):
        need = min(x["min_effect_advance_us"] for x in group["comparisons"]["oban"])
        illustration = cursor["fixed_stamp_pipeline_credit_ceiling_us"]
        scale.append(dict(group=group["group"], required_last_effect_advance_us=need,
                          stock_pipeline_illustration_us=illustration,
                          proportional_binary_credit_us=round(illustration * (total-size["binary"]) / total)))
    return dict(input_sha256=manifest, baseline_raw_sha256=hashes, args_text_bytes=total,
                zstd_version=subprocess.check_output([zstd, "--version"], text=True).strip(),
                zstd_level=1, compressed_transport_bytes=size, context_count=len(values),
                historical_scale=scale)


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--pg-bin", required=True, type=Path)
    p.add_argument("--zstd", default="zstd")
    args = p.parse_args()
    print(json.dumps(replay(args.pg_bin, args.zstd), indent=2, sort_keys=True))
