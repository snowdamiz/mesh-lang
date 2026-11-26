"""Build a source-only native staging bundle from retained/pinned fixtures."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile

REPLAY=Path(__file__).resolve().parent
LANE=REPLAY.parent
ROOT=LANE.parents[1]
STATE=ROOT/"target/linode-jobs-stepchange"
STOCK=ROOT/"target/linode-jobs-native/bundles/stock-3828f20d/source"
BASE="3828f20d2d6beb683868cce33e6dd1958365914b"
MAIN="dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17"
PRODUCT=("compiler/mesh-rt/src/jobs/store.rs", "compiler/mesh-rt/src/jobs/enqueue_schema.sql")

def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream,"sha256").hexdigest()

def checkout_source(revision, path):
    with subprocess.Popen(["git","-C",str(LANE/"source"),"archive",revision,
                           ".cargo","Cargo.toml","Cargo.lock","compiler","tools/skill/mesh",
                           "docs/security/dependency-policy.md","scripts"],stdout=subprocess.PIPE) as proc:
        subprocess.run(["tar","-xf","-","-C",str(path)],stdin=proc.stdout,check=True)
        assert proc.wait() == 0

def instrument(root):
    native=root/"tools/durable-jobs-bench/native"
    run=native/"run"
    s=run.read_text()
    start=s.index('env_args=(env -i')
    newline=s.index('\n',start)
    s=s[:newline]+'''\n    "MESH_JOBS_SET_BASED_ENQUEUE=${MESH_JOBS_SET_BASED_ENQUEUE:-0}"'''+s[newline:]
    run.write_text(s)
    arm=native/"arm.py"
    s=arm.read_text()
    needle='        if engine == "oban":\n'
    index=s.index(needle,s.index("        cluster.start()"))
    s=s[:index]+'''        if engine == "mesh":
            env["MESH_JOBS_SET_BASED_ENQUEUE"] = os.environ.get("MESH_JOBS_SET_BASED_ENQUEUE", "0")
'''+s[index:]
    index=s.index('        database = database_settings(cluster, engine)')
    s=s[:index]+'''        if engine == "mesh":
            statements = json.loads(cluster.sql(engine, """
                SELECT coalesce(json_object_agg(p.query, p.calls), '{}')
                FROM pg_stat_statements p
                WHERE p.dbid = (SELECT oid FROM pg_database WHERE datname=current_database())
                AND p.userid = (SELECT oid FROM pg_roles WHERE rolname='parity_producer')
                AND (p.query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'
                     OR p.query LIKE 'SELECT mesh_jobs.enqueue(%')
            """))
            bulk = sum(v for k,v in statements.items() if k.startswith("SELECT ordinal, receipt"))
            scalar = sum(v for k,v in statements.items() if k.startswith("SELECT mesh_jobs.enqueue("))
            enabled = env["MESH_JOBS_SET_BASED_ENQUEUE"] == "1"
            if profile in ("short", "noop", "slow"):
                if (enabled and bulk == 0) or (not enabled and (bulk or scalar == 0)):
                    raise RuntimeError("producer did not exercise the selected writer")
            h.write_json(out / "producer-writer.json", dict(enabled=enabled, bulk=bulk,
                                                            scalar=scalar, statements=statements))
'''+s[index:]
    arm.write_text(s)

def main(revision, archive):
    stage=LANE/f"stage-{revision[:12]}"
    stage.mkdir()
    strict=stage/"source"
    strict.mkdir()
    checkout_source(revision,strict)
    for name in ("proof/jobs-parity/linux-regression", "tools/durable-jobs-bench/native"):
        shutil.copytree(STOCK/name,strict/name,dirs_exist_ok=True)
    for name in ("jobs-parity.py","jobs-parity-profiles.json","ci-x86_64-linux-gnu-clang"):
        shutil.copy2(STOCK/"scripts"/name,strict/"scripts"/name)
    for name in ("fixture","screen.py","prove.py"):
        path=LANE/name
        if path.is_dir():
            shutil.copytree(path,stage/name)
        else:
            shutil.copy2(path,stage/name)
    strict_fixture=stage/"strict-fixture"
    strict_fixture.mkdir()
    frozen=STATE/"stock-3828f20d-amd64.tar.gz"
    with tarfile.open(frozen) as t:
        for name in ("main.mpl","worker_identity.mpl","worker_identity.c","mesh.toml"):
            (strict_fixture/name).write_bytes(t.extractfile("mesh-source/"+name).read())
    change=subprocess.check_output(["git","-C",str(LANE/"source"),"diff","--binary",BASE,revision,"--",*PRODUCT])
    pinned=dict(strict_archive=sha(frozen),source_revision=revision,strict_input_base=BASE)
    for version in ("v1","v2"):
        name=f"ordinary-trusted-{version}"
        request=json.loads((STATE/"requests"/f"{name}.json").read_text())
        sealed=Path(request["source_archive"])
        assert sha(sealed)==request["source_archive_sha256"], sealed
        path=stage/f"ordinary-{version}"
        path.mkdir()
        with tarfile.open(sealed) as t:
            t.extractall(path,filter="data")
        subprocess.run(["patch","--dry-run","-p1","-f"],cwd=path,input=change,check=True)
        subprocess.run(["patch","-p1","-f"],cwd=path,input=change,check=True)
        assert (path / PRODUCT[1]).read_bytes() == (strict / PRODUCT[1]).read_bytes()
        instrument(path)
        pinned[name]=dict(archive=sha(sealed),base=request["source_sha"])
    instrument(strict)
    (stage/"stage-inputs.json").write_text(json.dumps(pinned,indent=2,sort_keys=True)+"\n")
    with tarfile.open(archive,"x:gz") as t:
        for path in sorted(stage.iterdir()):
            t.add(path,arcname=path.name)
    print(sha(archive),archive)

if __name__=="__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("revision")
    p.add_argument("archive",type=Path)
    a=p.parse_args()
    main(a.revision,a.archive)
