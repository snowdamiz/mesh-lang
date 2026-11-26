"""Check all eight runner logs against the sealed fixture reconciliation."""
import argparse
import json
from pathlib import Path

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("runs", type=Path, nargs="+")
a = p.parse_args()
result = {}
for root in a.runs:
    document = json.loads((root / "ledger.json").read_text())
    row = json.loads((root / "row.json").read_text())
    assert row["runners"] == 8
    assert row["reconciled_jobs"] == len(document["jobs"]) == 1024
    assert row["committed_work_records"] == 2048
    assert row["peak_connections"] <= row["connection_cap"] == 96
    assert row["execution_contract"] == "ordinary-trusted-shared-process"
    runners = []
    for path in sorted(root.glob("runner-*.log")):
        lines = path.read_text().splitlines()
        status, = [json.loads(line) for line in lines if line.startswith("{")]
        calls, = [json.loads(line.removeprefix("STACK_CALLS ")) for line in lines
                  if line.startswith("STACK_CALLS ")]
        counts = calls["524288"]
        completed = status["outcomes"]["completed"]
        assert status["claimed"] == status["finished"] == completed
        assert status["drained"] and status["last_error"] is None and not status["undrained"]
        assert counts["reserve"] == counts["protect"] == counts["unmap"] and counts["live"] == 0
        assert calls["65536"]["reserve"] == 0
        if "retained" in root.name:
            assert counts["reserve"] == completed + 1 and counts["reset"] == 0
        else:
            assert counts["reserve"] <= completed + 1
            idle_at_exit = counts["reset"] - ((completed + 1) - counts["reserve"])
            assert 0 < idle_at_exit <= 8 * calls["owners"]
        runners.append(dict(log=path.name, completed=completed, status=status, calls=calls))
    assert len(runners) == row["runners"]
    assert sum(r["completed"] for r in runners) == 1024
    result[root.name] = dict(arm=row, runners=runners,
                             total_calls={key: sum(r["calls"]["524288"][key] for r in runners)
                                          for key in ("reserve", "protect", "reset", "unmap")})
print(json.dumps(result, indent=2))
