"""Seal the public Jobs ELF inputs and replay the sole controller's SETUP export."""
import argparse
import gzip
from hashlib import file_digest, sha256
from io import BytesIO
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[2]
PRODUCT = "tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition"
PILOT = "tools/durable-jobs-bench/results/2026-11-25-dense-grouped-refill"
FIXTURE = "b3557273d72be3802bf189b284e7caaf5f3caa0f"
EVIDENCE = "5670cc95e02f0587c98cf0fdbf41b99a1cac51f3"
COMPILER = "e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19"
GROUP_SCHEMA = "compiler/mesh-rt/src/jobs/grouped_schema.sql"
CLAIM_SHA = "d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45"
CASES = (
    "sql-cut", "checked-individual", "ambiguous-claim-commit", "ambiguous-application-write",
    "held-terminal", "lost-terminal", "lost-bulk", "cancellation", "held-teardown",
    "forced-teardown", "stop-partial",
)


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT)


def show(revision, path):
    return git("show", f"{revision}:{path}")


def digest(path):
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def replace_once(data, old, new):
    assert data.count(old) == 1, old
    return data.replace(old, new)


def inputs():
    names = (
        "fixture/main.mpl", "fixture/writes.mpl", "fixture/worker_identity.mpl",
        "fixture/worker_identity.c", "replay/semantic.py", "replay/identity.py", "replay/link.sh",
        "replay/native-setup.sh", "replay/wire.py", "replay/teardown.c", "replay/witness_fixture.py",
        "replay/product_arm.py", "replay/native-run",
    )
    files = {name: show(FIXTURE, f"{PRODUCT}/{name}") for name in names}
    backend = show(EVIDENCE, f"{PRODUCT}/closure-retry/backend.py")
    backend = replace_once(backend, b'    "product-d2": ("success_cohort", True),',
                           b'    "product-d2": ("success_cohort", True),\n'
                           b'    "product-checked-hot": ("set_based", True),')
    backend = replace_once(backend, b'    "group": "mesh_jobs.complete_batch_v1',
                           b'    "bounded": "mesh_jobs.claim_batch_bounded_v1(text,text,jsonb,bigint,integer,bigint)",\n'
                           b'    "group": "mesh_jobs.complete_batch_v1')
    backend = replace_once(backend, b"{GRANTS['claim']}, {GRANTS['group']}",
                           b"{GRANTS['claim']}, {GRANTS['bounded']}, {GRANTS['group']}")
    backend = replace_once(backend, b'("claim_admitted_v3", "claim_batch_set_v1"),',
                           b'("claim_admitted_v3", "claim_batch_bounded_v1"\n'
                           b'                      if os.environ["PRODUCT_COMPOSITION"] == "product-d2"\n'
                           b'                      else "claim_batch_set_v1"),')
    files["replay/backend.py"] = backend
    files["replay/wire.py"] = replace_once(
        files["replay/wire.py"],
        b'if kind == b"D" and b"FROM mesh_jobs.claim_batch_set_v1(" in query:',
        b'''if kind == b"D" and any(name in query for name in (
                        b"FROM mesh_jobs.claim_batch_set_v1(",
                        b"FROM mesh_jobs.claim_batch_bounded_v1(")):''',
    )
    semantic = files["replay/semantic.py"]
    assert semantic.count(b"mesh_jobs.claim_batch_set_v1(") == 2
    files["replay/semantic.py"] = semantic.replace(
        b"mesh_jobs.claim_batch_set_v1(", b"mesh_jobs.claim_batch_bounded_v1(")
    faults = show(EVIDENCE, f"{PILOT}/verify_candidate.py")
    faults = replace_once(faults, b'"""Untimed faults against the patched public b3557273 fixture and TLS relay."""',
                          b'"""Untimed faults against the public fixed-window Jobs runner."""')
    faults = replace_once(faults, b"from semantic import (Relay, arm, cluster", b"from semantic import (Relay, arm, claim_loss, cluster, handler_cut")
    faults = replace_once(faults, b'    os.environ["JOBS_GROUPED_WRITER"] = "refill8ms"',
                          b'''    os.environ["PRODUCT_COMPOSITION"] = "product-checked-hot"
    checked_individual(out / "checked-individual", executable)
    os.environ["PRODUCT_COMPOSITION"] = "product-d2"
    claim_loss(out / "ambiguous-claim-commit", executable)
    handler_cut(out / "ambiguous-application-write", executable, fault=True)''')
    faults = replace_once(faults, b'if __name__ == "__main__":\n',
                          (Path(__file__).with_name("individual.py").read_bytes() + b'\n\nif __name__ == "__main__":\n'))
    entry, cases = faults.split(b"    out.mkdir()\n")
    faults = entry + b"    out.mkdir()\n    try:\n" + b"".join(
        b"    " + line for line in cases.splitlines(keepends=True)
    ) + b"    finally:\n        redact_tokens(out)\n"
    files["replay/verify_grouped_writer.py"] = faults
    script = files["replay/native-setup.sh"]
    script = replace_once(script, b"^trusted-jobs-product-", b"^public-dense-fixed8-jobs-")
    script = replace_once(script, b'trap \'rm -rf -- "$CARGO_TARGET_DIR" "$TMPDIR"\' EXIT',
                          b'''seal() {
    local code="$?"
    printf '%s\\n' "$code" > "$out/native.exit"
    rm -rf -- "$CARGO_TARGET_DIR" "$TMPDIR"
}
trap seal EXIT''')
    script = replace_once(script, b'"$packet/replay/semantic.py"', b'"$packet/replay/verify_grouped_writer.py"')
    subprocess.run(["bash", "-n"], input=script, check=True)
    files["replay/native-setup.sh"] = script
    for name, data in files.items():
        if name.endswith(".py"):
            compile(data, name, "exec")
    return files


def verify_sql(data):
    candidate = data.split(b"\nREVOKE EXECUTE", 1)[0]
    candidate = candidate.replace(b"mesh_jobs.claim_batch_bounded_v1(", b"mesh_jobs.claim_batch_set_v1(", 1)
    assert sha256(candidate).hexdigest() == CLAIM_SHA


def pack(revision, output):
    output.mkdir(parents=True, exist_ok=False)
    revision = git("rev-parse", "--verify", f"{revision}^{{commit}}").decode().strip()
    verify_sql(show(revision, GROUP_SCHEMA))
    assert show(revision, "compiler/mesh-rt/src/jobs/schema.sql") == show(
        "1e70f18d624a841740d3bbb3d0fd09b269257f68", "compiler/mesh-rt/src/jobs/schema.sql")
    files = inputs()
    archive = output / "source.tar.gz"
    paths = (
        ".cargo", "Cargo.toml", "Cargo.lock", "compiler", "tools/skill/mesh",
        "scripts/ci-x86_64-linux-gnu-clang", "scripts/fixtures/jobs-role-grants.sql",
        "scripts/fixtures/jobs-group-success-grant.sql",
    )
    with (archive.open("xb") as stream, gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=0) as gz,
          tarfile.open(fileobj=gz, mode="w|") as packed, subprocess.Popen(
              ["git", "archive", "--format=tar", "--prefix=source/", revision, *paths],
              stdout=subprocess.PIPE, cwd=ROOT) as source):
        with tarfile.open(fileobj=source.stdout, mode="r|") as entries:
            for item in entries:
                assert item.isfile() or item.isdir(), item.name
                packed.addfile(item, entries.extractfile(item) if item.isfile() else None)
        assert source.wait() == 0
        for name, data in sorted(files.items()):
            item = tarfile.TarInfo(f"source/{PRODUCT}/{name}")
            item.size = len(data)
            item.mode = 0o755 if name.endswith((".sh", "/native-run")) else 0o644
            packed.addfile(item, BytesIO(data))
    archive_sha = digest(archive)
    script = files["replay/native-setup.sh"]
    (output / "native.sh").write_bytes(script)
    attempt = f"public-dense-fixed8-jobs-{archive_sha[:12]}"
    packet = dict(
        id=attempt + "-setup", mode="jobs", ready=True, cpus=[0, 1],
        requested_window_minutes=45, source_sha=revision, archive_sha256=archive_sha,
        native_script_sha256=sha256(script).hexdigest(), attempt=attempt,
        arguments=[attempt, revision, archive_sha],
        evidence_destination=str(output / "evidence"),
        remote_evidence=f"/var/lib/durable-jobs-bench/native/runs/{attempt}",
    )
    (output / "jobs.json").write_text(json.dumps(packet, indent=2, sort_keys=True) + "\n")
    check(output)
    print(digest(output / "jobs.json"))


def check(output, *, git_sources=True):
    packet = json.loads((output / "jobs.json").read_text())
    source = output / "source.tar.gz"
    script = (output / "native.sh").read_bytes()
    assert digest(source) == packet["archive_sha256"]
    assert sha256(script).hexdigest() == packet["native_script_sha256"]
    with tarfile.open(source) as entries:
        unpack = lambda name: entries.extractfile(f"source/{name}").read()
        if git_sources:
            for name in (GROUP_SCHEMA, "compiler/mesh-rt/src/jobs/runner.rs",
                         "compiler/mesh-rt/src/jobs/grouped_writer.rs", "compiler/mesh-rt/src/jobs/store.rs"):
                assert unpack(name) == show(packet["source_sha"], name)
        verify_sql(unpack(GROUP_SCHEMA))
        assert script == unpack(f"{PRODUCT}/replay/native-setup.sh")
    return packet


def replay(output, release_path, raw, *, git_sources=True):
    packet = check(output, git_sources=git_sources)
    release = json.loads(release_path.read_text())
    assert release["id"] == packet["id"] and release["exit_status"] == 0
    raw_sha = digest(raw)
    assert release.get("raw_sha256", raw_sha) == raw_sha
    with tarfile.open(raw) as entries, tarfile.open(output / "source.tar.gz") as source:
        def read(name):
            return entries.extractfile("./" + name).read()
        assert read("native.exit").strip() == b"0"
        mesh, witness = [json.loads(read(f"{name}.json")) for name in ("mesh", "witness")]
        for item in (mesh, witness):
            assert (item["system"], item["architecture"]) == ("Linux", "x86_64")
            assert item["version"] == packet["source_sha"]
            assert item["source_sha256"] == packet["archive_sha256"]
            assert item["compiler_sha256"] == COMPILER
        assert mesh["runtime_sha256"] == witness["runtime_sha256"]
        assert mesh["files"]["mesh"] != witness["files"]["mesh"]
        for name in ("main.mpl", "writes.mpl", "worker_identity.mpl", "worker_identity.c"):
            fixture = source.extractfile(f"source/{PRODUCT}/fixture/{name}").read()
            assert sha256(fixture).hexdigest() == mesh["files"][name]
        assert b"ELF64" in read("elf.txt") and b"X86-64" in read("elf.txt")
        cases = {name: read(f"semantic-root/semantic/{name}/result.json") for name in CASES}
    rows = {name: json.loads(value) for name, value in cases.items()}
    assert rows["checked-individual"]["terminal_xids"] == 2
    assert rows["ambiguous-claim-commit"]["held"]["effect"] is None
    assert rows["ambiguous-claim-commit"]["done"]["attempt"] == 2
    assert rows["ambiguous-application-write"]["app_insert_calls"] == 3
    for name, lost in (("held-terminal", False), ("lost-terminal", True)):
        row = rows[name]
        assert row["wire"]["idle"] and row["wire"]["receipt_dropped"] == lost
        assert row["available_while_held"] == len(row["started_while_held"]) == 3
        assert row["renewal_while_held"] > row["renewal_before"]
        assert row["independent_app_xids"] == 10
    assert rows["lost-bulk"]["bulk_submissions"] == 1
    assert rows["lost-bulk"]["token_stale_confirmations"] == 2
    assert rows["sql-cut"]["rolled_back"] and rows["sql-cut"]["wrong_token_untouched"]
    assert rows["sql-cut"]["expired_untouched"] and rows["sql-cut"]["legacy_mix"]
    assert rows["cancellation"]["cancel"]["state"] == "cancelled"
    assert rows["cancellation"]["retry"]["attempt"] == 2
    assert rows["held-teardown"]["done"]["state"] == "completed"
    assert rows["forced-teardown"]["done"]["state"] == "executing"
    assert rows["stop-partial"]["drained"]["state"] == "completed"
    return dict(packet=packet["id"], source=packet["source_sha"], elf_sha256=mesh["files"]["mesh"],
                witness_elf_sha256=witness["files"]["mesh"], runtime_sha256=mesh["runtime_sha256"],
                raw_sha256=raw_sha, release_sha256=digest(release_path),
                proofs={name: sha256(value).hexdigest() for name, value in cases.items()})


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    pack_cmd = sub.add_parser("pack")
    pack_cmd.add_argument("revision")
    pack_cmd.add_argument("output", type=Path)
    for name in ("check", "replay"):
        action = sub.add_parser(name)
        action.add_argument("output", type=Path)
        if name == "replay":
            action.add_argument("release", type=Path)
            action.add_argument("raw", type=Path)
    args = parser.parse_args()
    if args.command == "pack":
        pack(args.revision, args.output.resolve())
    elif args.command == "check":
        print(json.dumps(check(args.output), indent=2, sort_keys=True))
    else:
        print(json.dumps(replay(args.output, args.release, args.raw), indent=2, sort_keys=True))
