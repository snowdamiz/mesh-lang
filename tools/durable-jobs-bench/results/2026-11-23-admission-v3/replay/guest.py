#!/usr/bin/env python3
"""Run the pinned two-vCPU Linux protected fixture with each dispatcher mode."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import stat

from price import module

HERE = Path(__file__).resolve().parent
PRIOR = module(HERE.parent / "dense-settlement-pipeline/run.py")
base, frozen = PRIOR.base, PRIOR.frozen


WAIT_CHECK = '''

def record_waits(h, cluster, out, ns):
    ledger = json.loads(cluster.sql('mesh', f"""
        SELECT json_agg(json_build_array(a.worker_pid, j.id, a.xmin::text, e.xmin::text, j.xmin::text,
           floor(extract(epoch FROM pg_xact_commit_timestamp(e.xmin)) * 1e6)::bigint,
           floor(extract(epoch FROM pg_xact_commit_timestamp(j.xmin)) * 1e6)::bigint))
        FROM mesh_jobs.jobs j JOIN public.mesh_jobs_parity_attempts a
          ON (j.namespace,j.id,j.run)=(a.namespace,a.job_id,a.job_run)
        JOIN public.mesh_jobs_parity_effects e
          ON (j.namespace,j.id,j.run)=(e.namespace,e.job_id,e.job_run)
        WHERE j.namespace='{ns}' AND j.state='completed' AND j.attempt=1
    """))
    local_files = sorted(out.glob('wait-*.txt'))
    assert len(local_files) == cluster.runners
    execs, waits = {}, {}
    for path in local_files:
        for line in path.read_text().splitlines():
            kind, pid, status, stamp = line.split()
            pid, status, stamp = int(pid), int(status), int(stamp)
            events = execs if kind == 'exec' else waits
            assert kind in ('exec','wait') and pid not in events and status == 0, line
            events[pid] = (stamp, path.name)
    assert execs.keys() == waits.keys() == {row[0] for row in ledger} and len(ledger) == 1024
    for pid, _, first_xid, second_xid, terminal_xid, effect_us, terminal_us in ledger:
        spawn, owner = execs[pid]
        wait, reaper = waits[pid]
        assert owner == reaper and len({first_xid, second_xid, terminal_xid}) == 3
        assert spawn <= effect_us * 1000 <= wait <= (terminal_us + 1) * 1000, (pid, spawn, effect_us, wait, terminal_us)
    h.write_json(out / 'consuming-wait-ledger.json', dict(rows=ledger, spawn=execs, wait=waits))


'''


def trial_source(traced):
    source = PRIOR.clock.trial_source(False).decode()
    source = base.replace(source, "    lane.audit_environment(env, out)",
        """    if arm == 'mesh':
        if lane.variant == 'pipeline':
            subject.command = ['/proof/mesh-pipeline']
            env['MESH_DIAGNOSTIC_SETTLEMENT_PIPELINE'] = '1'
            env['MESH_DIAGNOSTIC_FROZEN_DENSE_WORKER'] = '1'
        else:
            subject.command = ['/proof/mesh-v3']
            env['MESH_JOBS_ADMISSION_V3'] = lane.variant
    lane.audit_environment(env, out)""")
    source = base.replace(source, '    lane.audit_end(h, cluster, arm, out, result)',
        """    lane.audit_end(h, cluster, arm, out, result)
    if arm == 'mesh':
        raw = json.loads((out / 'raw.json').read_text())
        ns = raw['namespace']
        terminal = json.loads(cluster.sql(arm, f'''
            SELECT json_build_object('jobs',count(*), 'xids',count(distinct xmin::text))
            FROM mesh_jobs.jobs WHERE namespace='{ns}' AND state='completed'
        '''))
        assert terminal['jobs'] == result['completed']
        if lane.variant != 'cohort':
            assert terminal['jobs'] == terminal['xids'], terminal
        calls = json.loads(cluster.sql(arm, '''
            SELECT json_agg(json_build_array(query,calls,rows) ORDER BY query)
            FROM pg_stat_statements WHERE dbid=(SELECT oid FROM pg_database WHERE datname='mesh')
               AND userid IN ('parity_subject'::regrole,'compiled_caller'::regrole)
        '''))
        admitted = sum(n for q,n,r in calls if 'FROM mesh_jobs.claim_admitted_v3(' in q)
        cohorts = sum(n for q,n,r in calls if 'FROM mesh_jobs.complete_admitted_v3(' in q)
        assert (admitted > 0) == (lane.variant in ('strict', 'cohort')), calls
        assert (cohorts > 0) == (lane.variant == 'cohort'), calls
        h.write_json(out / 'admission-ledger.json', dict(mode=lane.variant,
                     terminal=terminal, admitted=admitted, cohorts=cohorts, statements=calls))
""")
    # The pipeline comparator runs a different coordinator ELF and explicitly
    # execs /proof/mesh-current. The three v3 modes all exec their own same ELF.
    source = base.replace(source, "    shared_broker = True", """    shared_broker = True

    def run(self, env, index, log):
        if TRACE_WAIT:
            env = dict(env, LD_PRELOAD='/proof/v3-wait.so',
                       MESH_ADMISSION_WAIT_RECEIPT=str(log.parent / f'wait-{index}.txt'))
        process = super().run(env, index, log)
        if self.engine == 'mesh' and lane.variant == 'pipeline':
            process.fixture_executable = '/proof/mesh-current'
        return process
""")
    if traced:
        source = base.replace(source, '    h.write_json(out / "metrics.json", result)',
            '    record_waits(h, cluster, out, ns)\n    h.write_json(out / "metrics.json", result)')
    source = f"TRACE_WAIT = {traced!r}\n" + WAIT_CHECK + source
    compile(source, "trial.py", "exec")
    return source.encode()


def package(image, profile, arms, mesh, mesh_sha, assets, modes, binary, traced):
    frozen.pack(image, profile, arms, mesh, mesh_sha, assets, base_path=base.BASE,
                init_path=base.BASE.parent / "stock-jobs-smoke/init")
    with image.open("rb") as stream:
        stream.seek((base.BASE.stat().st_size + 3) & ~3)
        config, = (json.loads(data) for name, _, data in frozen.members(stream.read())
                   if name == "proof/config.json")
    reference = base.WORK / "canonical-inetrc/pilots/dense-fixed-01/evidence/config.json"
    assert config["hashes"] == json.loads(reference.read_text())["hashes"]
    assert config["profile"] == "noop" and config["full_input_warmup"]
    config.update(plan=[(name, "mesh", name) for name in modes], series="jobs-admission-v3",
                  resolver_evidence=PRIOR.clock.resolver_identity())
    overlay = bytearray()

    def put(name, data, mode=0o644):
        config["hashes"][name] = frozen.digest(data)
        frozen.add(overlay, name, stat.S_IFREG | mode, data)

    put("proof/trial.py", trial_source(traced))
    put("proof/proof_transport.py", base.transport_source())
    put("proof/lane_guest.py", (PRIOR.clock.HERE / "lane_guest.py").read_bytes())
    put("proof/mesh-v3", binary.read_bytes(), 0o755)
    if traced:
        put("proof/v3-wait.so", (HERE / "wait_observer.so").read_bytes(), 0o755)
    if "pipeline" in modes:
        put("proof/mesh-pipeline", (PRIOR.HERE / "mesh").read_bytes(), 0o755)
    frozen.add(overlay, "proof/config.json", stat.S_IFREG | 0o644, json.dumps(config).encode())
    frozen.add(overlay, "TRAILER!!!", 0)
    with image.open("ab") as stream:
        stream.write(b"\0" * (-stream.tell() % 4))
        stream.write(overlay)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("name")
    parser.add_argument("--modes", nargs="+", default=["stock", "strict", "cohort", "pipeline"],
                        choices=["stock", "strict", "cohort", "pipeline"])
    parser.add_argument("--binary", type=Path, default=HERE / "mesh")
    parser.add_argument("--trace-wait", action="store_true")
    args = parser.parse_args()
    assert re.fullmatch("[a-z0-9][a-z0-9-]*", args.name)
    args.output = HERE / args.name
    assert not args.output.exists(), args.output
    args.profile, args.arms = "noop", ["mesh", "oban"]
    args.mesh, args.mesh_sha256, args.assets = base.WORK / "mesh-frozen-final", base.PIN, base.ASSETS
    boot = HERE / "boot"
    if not boot.exists():
        shutil.copy2(base.WORK / "canonical/boot", boot)
    base.canonical.pack = lambda *argv: package(*argv, args.modes, args.binary, args.trace_wait)
    transport = vars(base.proof_transport).copy()
    exec(compile(base.transport_source(), "proof_transport.py", "exec"), transport)
    base.canonical.read_serial = transport["read_serial"]
    os.environ["TMPDIR"] = str(HERE / "tmp")
    print(f"{args.name} waiting for queue-performance.lock", flush=True)
    try:
        status = base.canonical.run(args)
        if status and (args.output / "evidence/failure.log").is_file():
            print((args.output / "evidence/failure.log").read_text(), flush=True)
        (args.output / "exit.json").write_text(json.dumps({"status": status}) + "\n")
        return status
    finally:
        (args.output / "guest.cpio").unlink(missing_ok=True)


if __name__ == "__main__":
    raise SystemExit(main())
