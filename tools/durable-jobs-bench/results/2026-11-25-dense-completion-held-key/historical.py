"""Check exact archived nested stock SQL and re-run the parent workload reduction."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / "2026-11-25-dense-factorial-interaction"
NATIVE = HERE.parent / "2026-11-25-dense-logical-grant-key-native"
FACTORIAL = HERE.parent / "2026-11-25-dense-fixed8-logical-factorial"


def read(path):
    return json.loads(path.read_bytes())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify():
    historical = subprocess.check_output([sys.executable, "-B", PARENT / "settlement_scans.py"])
    assert historical == (PARENT / "settlement-scans.json").read_bytes()
    result = json.loads(historical)
    stock = (HERE / "stock.sql").read_text().split("$$")[1]
    rendered = stock.replace("token", "lease_marker")
    for lane, row in result["sampled_plans"].items():
        path, = (NATIVE / "evidence/native/blocks").glob(f"00-*/measured-{lane}/measured-mesh/raw.tar.gz")
        assert digest(path) == row["raw_sha256"]
        with tarfile.open(path) as bundle:
            plans = json.load(bundle.extractfile("adapter/grant-plans.json"))
        grouped = [plan for plan in plans if "WITH held AS MATERIALIZED" in plan["query"]]
        assert len(grouped) == sum(row["width_counts"].values())
        assert {plan["query"] for plan in grouped} == {rendered}
    packet = read(FACTORIAL / "controller-packet-retry.json")
    assert packet["candidate_sha256"] == digest(NATIVE / "candidate.sql")
    assert packet["elf_sha256"] == "d56d0e6112a94ff601bebf6f777061068ed2fe0f2f2da586963dd6e1326a220a"
    print("62 archived nested bodies, released fixed8+d388 pins and six warmed River reductions verified")


if __name__ == "__main__":
    verify()
