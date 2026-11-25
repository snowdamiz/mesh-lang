"""Compare the stock scalar lock and UPDATE plans in released SLOW and DENSE diagnostics."""
from collections import Counter
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import subprocess
import tarfile

from settlement_scans import nodes, read

ROOT = Path(__file__).resolve().parents[4]
RESULTS = Path("tools/durable-jobs-bench/results")
PROFILES = {
    "dense": ("4d8c05545", "2026-11-25-public-dense-stock-logical-key", 1024),
    "slow": ("7df5b961c", "2026-11-25-public-slow-width16-logical-key-writer-recovery", 512),
}
STATEMENTS = {"lock": "-1040032035499382046", "update": "-6581132960969787492"}


def blob(commit, path):
    return subprocess.check_output(["git", "show", f"{commit}:{path}"], cwd=ROOT)


def replay(profile):
    commit, dirname, count = PROFILES[profile]
    directory = RESULTS / dirname
    binding = json.loads(blob(commit, directory / "continuation/qualification.json"))
    assert binding["plan_gate"]["passed"]
    result = {}
    for lane in ("plan-stock", "plan-key"):
        member = f"measured-{lane}/measured-mesh"
        if profile == "slow":
            digest = binding["raw_sha256"][member]
        else:
            digest = binding["arms"][lane]["raw_sha256"]
        path = directory / "evidence/native/blocks" / Path(binding["block"]).name / member / "raw.tar.gz"
        data = blob(commit, path)
        assert sha256(data).hexdigest() == digest
        with tarfile.open(fileobj=BytesIO(data)) as raw:
            assert read(raw, "status.json")["exit_code"] == 0
            jobs = read(raw, "adapter/ledger.json")["jobs"]
            assert len(jobs) == len({j["settled_xid"] for j in jobs}) == count
            assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid", "settled_xid")}) == 3*count
            plans = read(raw, "adapter/grant-plans.json")
            stats = read(raw, "adapter/grant-closed.json")["statements"]
        by_statement = {}
        for name, query_id in STATEMENTS.items():
            matches = [r for r in plans if str(r["query_id"]) == query_id]
            counter, = [s for s in stats if s["role"] == "parity_subject" and str(s["queryid"]) == query_id]
            assert len(matches) == counter["calls"] == counter["rows"] == count
            indexes = Counter()
            removed = hits = 0
            for row in matches:
                assert row["session_role"] == "parity_subject"
                scan, = [n for n in nodes(row["plan"]) if n.get("Relation Name") == "jobs" and
                         "Scan" in n["Node Type"]]
                assert scan["Node Type"] == "Index Scan" and scan["Actual Loops"] == scan["Actual Rows"] == 1
                cond = scan["Index Cond"]
                assert "jobs.namespace =" in cond
                indexes[scan["Index Name"], "jobs.id =" in cond] += 1
                removed += scan["Rows Removed by Filter"]
                hits += scan["Shared Hit Blocks"]
            by_statement[name] = dict(calls=count, filtered_rows=removed, scan_hits=hits, paths=[
                dict(index=idx, id_in_condition=has_id, samples=n)
                for (idx, has_id), n in sorted(indexes.items())])
        result[lane] = dict(raw_sha256=digest, statements=by_statement)
    return result


if __name__ == "__main__":
    print(json.dumps({p: replay(p) for p in PROFILES}, indent=2, sort_keys=True))
