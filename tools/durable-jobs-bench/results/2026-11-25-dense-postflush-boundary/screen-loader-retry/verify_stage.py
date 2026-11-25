"""Verify the actual arm loader under the reduced UID, without starting a runner."""
import hashlib
import importlib.util
import json
import os
import signal
from pathlib import Path
import stat
import subprocess
import sys

HERE = Path(__file__).resolve().parent
HELPERS = ("verify_stage.py", "arm_overlay.py", "boundary.py", "native-arm.py", "topology_arm.py", "identity.py")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def load_stage():
    require(sys.platform == "linux" and os.getuid() == os.geteuid() == 65534 and sys.dont_write_bytecode,
            "expected Linux nobody and Python -B")
    for path in (HERE.parent, HERE):
        info = path.stat()
        require(info.st_uid == 0 and stat.S_IMODE(info.st_mode) == 0o711, "stage directory mode or owner changed")
        try:
            list(path.iterdir())
        except PermissionError:
            pass
        else:
            raise ValueError(f"stage directory is listable: {path}")
    for name in HELPERS:
        info = (HERE / name).lstat()
        require(info.st_uid == 0 and stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o644,
                f"unexpected helper mode or owner: {name}")
    spec = importlib.util.spec_from_file_location("postflush_overlay", HERE / "arm_overlay.py")
    overlay = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(overlay)
    return overlay


def load_arm(overlay, source, module, mode):
    app = overlay.load_app(module, mode)
    for helper, name in ((app, "native-arm.py"), (app.topology, "topology_arm.py"), (app.identity, "identity.py"),
                         (overlay.boundary, "boundary.py")):
        require(Path(helper.__file__) == HERE / name, f"wrong helper origin: {name}")
    arm = app.load_arm(source)
    native = source / "tools/durable-jobs-bench/native"
    require(Path(arm.__file__) == native / "arm.py"
            and Path(arm.ledger.__file__) == native / "ledger.py"
            and arm.Cluster.__module__ == overlay.__name__
            and arm.PROFILES["noop"] == app.DENSE, "frozen arm, second-level imports or cluster overlay changed")
    return arm


def activation(overlay, arm, module, mode, workspace):
    scratch, evidence = workspace / "cluster", workspace / "evidence"
    scratch.mkdir()
    evidence.mkdir()
    cluster = arm.Cluster(scratch, evidence, 96)
    cluster.runners, cluster.lanes, cluster.slots, cluster.hold_ms = 2, 16, 64, 0
    try:
        try:
            cluster.start()
            cluster.prepare_database("mesh")
            cluster.sql("mesh", """
              CREATE TABLE public.postflush_probe (value int);
              GRANT INSERT ON public.postflush_probe TO compiled_caller;
              CREATE FUNCTION public.postflush_mapping() RETURNS json
                LANGUAGE SQL SECURITY DEFINER SET search_path = pg_catalog
                AS $$ SELECT json_build_object('preload', current_setting('session_preload_libraries'),
                    'maps', pg_read_file('/proc/self/maps', 0, 1048576, false)) $$;
              REVOKE ALL ON FUNCTION public.postflush_mapping() FROM PUBLIC;
              GRANT EXECUTE ON FUNCTION public.postflush_mapping() TO compiled_caller;
            """)
            argv, env = cluster.psql("mesh")
            argv[argv.index("-U") + 1] = "compiled_caller"
            output = subprocess.check_output(argv, env=env, text=True, timeout=15, input="""
              BEGIN;
              INSERT INTO public.postflush_probe VALUES (1);
              SELECT json_build_object('pid', pg_backend_pid(), 'xid', pg_current_xact_id()::text,
                'tls', (SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()),
                'mode', current_setting('mesh_postflush.enabled'),
                'mapping', public.postflush_mapping());
              COMMIT;
            """)
            observation = json.loads(output)
            cluster.boundary["end"] = cluster.anchor("mesh")
        finally:
            cluster.stop()
    except Exception as error:
        raise ValueError(str(error).replace(cluster.password, "<redacted>")) from None
    preload = observation["mapping"]["preload"]
    require(observation["tls"] is True and observation["mode"] == mode and preload in {str(module), f'"{module}"'},
            "TLS session preload or hook setting differs")
    mapped = [line.split(maxsplit=5) for line in observation["mapping"]["maps"].splitlines() if "postflush.so" in line]
    module_stat = module.stat()
    require(mapped and all(len(line) == 6 and line[5] == str(module)
            and int(line[4]) == module_stat.st_ino
            and tuple(int(n, 16) for n in line[3].split(":")) ==
            (os.major(module_stat.st_dev), os.minor(module_stat.st_dev)) for line in mapped)
            and sum(int(line[2], 16) == 0 for line in mapped) == 1, "module pathname, inode or load count differs")
    pid = observation["pid"]
    exports = list(evidence.glob("postflush-*.log"))
    require([file.name for file in exports] == [f"postflush-{pid}.log"], "unexpected callback export roster")
    _, _, samples = overlay.boundary.parse_export(exports[0].read_text(), pid, mode == "on")
    require(samples.keys() == ({int(observation["xid"])} if mode == "on" else set()),
            "active commit did not match or inactive hook sampled")
    meta = json.loads((evidence / "postflush-arm.json").read_text())
    bounds = overlay.boundary.validate_clock(meta["start"], meta["end"])
    require(bounds[0] < int(observation["xid"]) < bounds[1]
            and "postflush evidence" not in (evidence / "postgres.log").read_text(),
            "wrong transaction identity or backend reported an export error")
    return dict(mode=mode, loaded_images=1, samples=len(samples), tls=True)

def main():
    probe, source, module, expected, *extra = sys.argv[1:]
    source, module = Path(source), Path(module)
    overlay = load_stage()
    checksum = digest(module)
    require(checksum == expected, "probe module hash changed")
    if probe == "imports":
        for mode in ("off", "on"):
            load_arm(overlay, source, module, mode)
        observations = []
    elif probe == "activation":
        def terminate(signum, _frame):
            raise SystemExit(128 + signum)

        signal.signal(signal.SIGTERM, terminate)
        mode, workspace = extra
        arm = load_arm(overlay, source, module, mode)
        observations = [activation(overlay, arm, module, mode, Path(workspace))]
    else:
        raise ValueError("invalid stage probe")
    require(digest(module) == checksum, "probe module changed")
    print(json.dumps(dict(probe=probe, uid=os.getuid(), dont_write_bytecode=sys.dont_write_bytecode,
                          stage_unlistable=True, second_level_imports=True, module_path=str(module),
                          module_sha256=checksum, observations=observations), sort_keys=True))


if __name__ == "__main__":
    main()
