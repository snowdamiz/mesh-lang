"""PostgreSQL 17.11 correctness replay. No timing or actual plan execution."""
from pathlib import Path
from tempfile import TemporaryDirectory
import json
import subprocess
import sys

from contract import DEPTHS, HERE, REVISION, ROOT, definitions
from probe import CALL, GUARD, SESSION, active, history, proc_identity, seed, semantics, signature


def check(bin):
    candidate = HERE.with_name("2026-11-25-empty-recovery") / "candidate.sql"
    sql_defs = definitions(candidate.read_bytes())

    def git(path):
        return subprocess.check_output(["git", "-C", ROOT, "show", f"{REVISION}:{path}"], text=True)

    def invoke(args, statement=None):
        result = subprocess.run(args, input=statement, text=True, capture_output=True, timeout=45)
        if result.returncode:
            raise RuntimeError(result.stderr)
        return result.stdout.strip()

    jobs = "compiler/mesh-rt/src/jobs"
    schema = "".join(git(f"{jobs}/{name}.sql") for name in
                     ("schema", "drain_schema", "cron_schema", "enqueue_schema"))
    grants = git("scripts/fixtures/jobs-role-grants.sql").split("-- Optional producer fast path.")[0]
    for role in ("producer", "worker", "operator", "inspector"):
        login = "subject" if role == "worker" else role
        grants = grants.replace(f':"{role}"', f"parity_{login}")
    receipts = {}
    with TemporaryDirectory(dir=ROOT / "target", prefix="native-er-proof-") as tmp:
        tmp = Path(tmp)
        invoke([bin/"initdb", "-D", tmp/"db", "-U", "parity_owner", "-A", "trust", "--no-locale", "-E", "UTF8"])
        ctl = [bin/"pg_ctl", "-D", tmp/"db", "-w"]
        invoke(ctl + ["-l", tmp/"pg.log", "-o", f"-c listen_addresses='' -c track_commit_timestamp=on -k {tmp}", "start"])
        try:
            cmd = [bin/"psql", "-XqAt", "-v", "ON_ERROR_STOP=1", "-U", "parity_owner", "-h", tmp, "-d", "postgres"]
            sql = lambda statement: invoke(cmd, statement)
            assert sql("SHOW server_version_num") == "170011"
            sql("CREATE ROLE parity_subject; CREATE ROLE parity_producer; "
                "CREATE ROLE parity_operator; CREATE ROLE parity_inspector;" + schema + grants)
            reset = "TRUNCATE mesh_jobs.batch_context_blocks, mesh_jobs.jobs;\n"
            for name, definition in sql_defs.items():
                sql(reset + definition)
                proc_identity(sql, definition)
                sql(history(0))
                receipt = receipts[name] = dict(semantics=semantics(sql, cmd, None), history={})
                for depth in DEPTHS:
                    sql(reset + history(depth))
                    receipt["history"][str(depth)] = signature(sql, depth)
                    sql(seed(5, True))
                    assert sql(SESSION + CALL) == "0"
                    assert active(sql) == {**{str(i): "available" for i in range(1,6)},
                                           "1001":"scheduled", "1002":"scheduled",
                                           "1003":"scheduled", "1004":"available"}
                    assert sql(GUARD) == "t"
        finally:
            invoke(ctl + ["-m", "fast", "stop"])
    assert receipts["stock"] == receipts["candidate"]
    return dict(postgres="17.11", timed=False, **receipts)


if __name__ == "__main__":
    print(json.dumps(check(Path(sys.argv[1]).resolve()), sort_keys=True, indent=2))
