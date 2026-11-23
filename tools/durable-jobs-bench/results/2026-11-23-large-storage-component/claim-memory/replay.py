"""Reconcile the captured PostgreSQL claim, settlement and query plan measurements."""
import hashlib
import json
from pathlib import Path
from statistics import median
import tarfile

HERE = Path(__file__).resolve().parent
ARCHIVE_SHA = "9271b909d2b96891a181c8155f9c622636301f87d2da11f0b066522132e1984a"
INPUT_SHA = "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1"
ARMS = {"stock-a", "header", "memory", "stock-b"}


def replay():
    path = HERE / "observations.tar.gz"
    assert hashlib.sha256(path.read_bytes()).hexdigest() == ARCHIVE_SHA
    with tarfile.open(path) as archive:
        def data(name):
            return json.load(archive.extractfile(name))

        inputs = data("timed/inputs.json")
        assert inputs["large_sha256"] == INPUT_SHA
        assert len(inputs["orders"]) == 4
        rows = {arm: [] for arm in sorted(ARMS)}
        first = data("timed/1-stock-a.json")["contexts"]
        assert len(first) == len({r["args_sha256"] for r in first}) == 64
        for index, order in enumerate(inputs["orders"], 1):
            assert len(order) == 4 and set(order) == ARMS
            for arm in order:
                row = data(f"timed/{index}-{arm}.json")
                assert row["rows_reconciled"] == 64 and row["contexts"] == first
                settings = row["settings"]
                assert settings["server_version_num"] == "170011"
                assert settings["fsync"] == settings["synchronous_commit"] == "on"
                assert int(settings["work_mem"]) == (16384 if arm == "memory" else 4096)
                rows[arm].append(row)
        summary = {}
        for operation in ("claim", "settle", "renew", "state_update"):
            cpu = {arm: [sum(r[operation]["backend_cpu_ms"].values()) for r in records]
                   for arm, records in rows.items()}
            wall = {arm: [r[operation]["wall_ms"] for r in records]
                    for arm, records in rows.items()}
            summary[operation] = {
                "median_backend_cpu_ms": {arm: round(median(v), 3) for arm, v in cpu.items()},
                "median_wall_ms": {arm: round(median(v), 3) for arm, v in wall.items()},
                "paired_cpu_ms_saved_vs_faster_stock": {
                    arm: [round(min(a, b) - c, 3) for a, b, c in
                          zip(cpu["stock-a"], cpu["stock-b"], cpu[arm], strict=True)]
                    for arm in ("memory", "header")},
                "wall_wins_vs_both": {arm: sum(c < min(a, b) for a, b, c in
                        zip(wall["stock-a"], wall["stock-b"], wall[arm], strict=True))
                        for arm in ("memory", "header")}}

        plans = data("plans.json")
        assert plans["input_sha256"] == INPUT_SHA
        assert [r["work_mem_mb"] for r in plans["plans"]] == [4, 16, 8, 6, 5, 4]
        blocks = []
        for row in plans["plans"]:
            root = row["explain"]["Plan"]
            read, written = root["Temp Read Blocks"], root["Temp Written Blocks"]
            assert root["Actual Rows"] == 8
            assert (read, written) == ((513, 513) if row["work_mem_mb"] == 4 else (0, 0))
            blocks.append([row["work_mem_mb"], read, written])
        key = data("effect-key/result.json")
        assert key["seed"] == INPUT_SHA
        hits = {(r["arm"], r["query"]): r["blocks"]["toast_hit"] for r in key["cases"]}
        assert len(hits) == 9
        for arm in ("stock", "bound", "stock-repeat"):
            assert hits[arm, "direct"] == hits[arm, "plpgsql"] == 0
            assert hits[arm, "row"] == (0 if arm == "bound" else 17024)
    return {"jobs_per_arm": 64, "arms": 16, "operations": summary,
            "work_mem_mb_and_temp_blocks_read_written": blocks}


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
