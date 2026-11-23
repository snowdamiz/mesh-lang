"""Run the original eight-row locking query repeatedly on one disposable backlog."""
import fcntl
import json
from pathlib import Path
import sys
import tempfile

from run import ROOT, restore


def main(out):
    out.mkdir()
    with tempfile.TemporaryDirectory(dir=ROOT / "target", prefix="jp-") as name:
        work = Path(name)
        pg = restore(work)
        jobs, seed = pg.profile_jobs()
        schema = (ROOT / "compiler/mesh-rt/src/jobs/schema.sql").read_text()
        with (ROOT / "target/queue-performance.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            cluster = pg.Cluster(work, out)
            try:
                cluster.start()
                with cluster.connect("postgres") as db:
                    db.simple(schema)
                    db.simple("ALTER TABLE mesh_jobs.jobs ALTER args SET STORAGE EXTERNAL")
                    pg.seed(db, "plans", jobs["large"])
                    receipt = {"input_sha256": seed, "plans": []}
                    for megabytes in (4, 16, 8, 6, 5, 4):
                        db.params("BEGIN")
                        db.params(f"SET LOCAL work_mem='{megabytes}MB'")
                        query = "EXPLAIN (ANALYZE, BUFFERS, SETTINGS, FORMAT JSON, TIMING OFF) " + pg.CLAIM
                        result = json.loads(pg.scalar(db, query, "plans", "default", pg.WORKERS, 30000, 8, 100, "false"))[0]
                        assert result["Plan"]["Actual Rows"] == 8
                        db.params("ROLLBACK")
                        assert pg.scalar(db, "SELECT count(*) FROM mesh_jobs.jobs WHERE attempt=0") == "64"
                        receipt["plans"].append({"work_mem_mb":megabytes,"explain":result})
                    pg.json_write(out / "plans.json", receipt)
            finally:
                cluster.stop()


if __name__ == "__main__":
    main(Path(sys.argv[1]).resolve())
