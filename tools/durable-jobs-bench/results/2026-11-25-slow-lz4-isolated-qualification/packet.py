#!/usr/bin/env python3
"""Seal, execute under the sole controller's grant, and replay the slow codec transfer."""
import argparse
from copy import deepcopy
from datetime import datetime, timedelta, timezone
import fcntl
import gzip
import json
from pathlib import Path
import re
import shlex
import sys
import tarfile
from uuid import uuid4

from strict import verify_codec, write_landmarks

HERE = Path(__file__).resolve().parent
OLD = HERE.parent / "2026-11-23-slow-storage-transfer"
SUITE = HERE.parents[1]
ROOT = SUITE.parents[1]
ID = HERE.name
STATE = ROOT / "target/linode-jobs-stepchange"
QUEUE = STATE / "requests"
LANE = ROOT / "target/slow-lz4-isolated-qualification"
OUT = HERE / "evidence/native"
SETUP = HERE / "controller-packet.json"
TIMING = HERE / "timing-packet.json"
STAGE = LANE / "native-stage.tar"
PG_NAME = "pgsql-17.11-lz4-537512904744"
NATIVE = Path("/var/lib/durable-jobs-bench/native")
STOCK = NATIVE / "toolchain/pgsql-17.11"
STOCK_BOOTSTRAP = SUITE / "native/bootstrap"
BOOT_LOG = ROOT / "target/linode-jobs-native/bootstrap-oban-v2.log"
HOST_FILES = ("build-pg.sh", "bridge.sh", "native-run", "overlay.py", "strict.py", "stock.sha256")
SOURCES = {
    "postgresql-17.11.tar.bz2": "dd27f2b3c59e73ed14aa3324901242bf69a032a6347805f274e6260322d42979",
    "lz4-1.10.0.tar.gz": "537512904744b35e232912055ccf8ec66d768639ff3abe5788d90d792ec5f48b",
}
CLOCKS = ("submit_to_last_settlement_ms", "commit_to_last_settlement_ms")

sys.path.insert(0, str(OLD / "replay"))
import native as original
import storage

matrix = original.matrix
digest = matrix.digest
read = matrix.read_json
write = matrix.write_json
require = matrix.require


def original_config():
    cfg = original.check()
    request = read(OLD / "request.json")
    frozen = request["frozen_candidate"]
    require(request["state"] == "hold" and request["id"] == "ordinary-slow-storage-transfer",
            "expected the original compatibility hold")
    require(cfg["orders"] == frozen["orders"] and cfg["cpus"] == frozen["cpus"]
            and cfg["profiles"] == ["slow"] and len(cfg["lanes"]) == 8,
            "original rotations or lanes changed")
    require(digest(STOCK_BOOTSTRAP) == "f209b1e6e263c41565db172baa4149efe1e00ce282d312c5ef22325bb92a694e",
            "original userspace bootstrap source changed")
    batch = OLD / "frozen/canonical-batch.json.gz"
    canonical = read(OLD / "frozen/canonical-input.json")
    require(digest(batch) == "6d25c3dd06227662e5913eceb7745517b11b132af08a0e1fbaced0bcf6e72b4e"
            and storage.sha(gzip.decompress(batch.read_bytes())) == canonical["batch_sha256"]
            and canonical["sha256"] == request["canonical_input_sha256"],
            "frozen canonical input differs")
    if BOOT_LOG.exists():
        require(digest(BOOT_LOG) == cfg["postgres"]["bootstrap_sha256"], "bootstrap log changed")
    return cfg


def stage_files():
    files = {name: HERE / name for name in HOST_FILES}
    files.update({name: OLD / "replay" / name for name in ("storage.py", "slow-river.sql")})
    files.update({name: LANE / "sources" / name for name in SOURCES})
    return files


def build_stage():
    files = stage_files()
    for name, sha in SOURCES.items():
        require(digest(files[name]) == sha, f"changed upstream source archive {name}")
    require(not STAGE.exists(), "stage path already exists")
    with tarfile.open(STAGE, "x:", format=tarfile.USTAR_FORMAT) as package:
        for name, path in sorted(files.items()):
            entry = tarfile.TarInfo(name)
            entry.mode = 0o755 if name.endswith(".sh") or name == "native-run" else 0o644
            entry.size = path.stat().st_size
            with path.open("rb") as stream:
                package.addfile(entry, stream)


def seal_setup():
    config = original_config()
    build_stage()
    paths = [HERE / x for x in (*HOST_FILES, "native-build.sh", "packet.py")]
    paths += [OLD / "request.json", OLD / "frozen/native-config.json", STOCK_BOOTSTRAP]
    paths += [SUITE / name for name in ("worker.py", "run.py", "verify.py")]
    command = f"python3 -B {shlex.quote(str(Path(__file__).resolve()))}"
    write(SETUP, {
        "id": ID, "kind": "SETUP", "ready": True, "requested_window_minutes": 35,
        "qualification_only": True, "timing_arms": 0, "diagnostic_arms": 8,
        "standing_authority": "Mesh benchmark campaign, 2026-11-25T16:18:48Z",
        "supersedes_grant": None, "original_hold": "ordinary-slow-storage-transfer",
        "stage_sha256": digest(STAGE), "stage": str(STAGE),
        "stage_members": {name: digest(path) for name, path in stage_files().items()},
        "sources": SOURCES, "workload_sha256": read(OLD / "request.json")["canonical_input_sha256"],
        "order": config["orders"][0], "cpus": config["cpus"],
        "evidence_destination": str(OUT / "setup"),
        "local_sha256": {str(path.relative_to(ROOT)): digest(path) for path in paths},
        "offline_check_command": command + " check",
        "setup_command": command + " setup",
        "verify_setup_command": command + " replay-setup",
    })


def check():
    config = original_config()
    packet = read(SETUP)
    require(packet["id"] == ID and packet["qualification_only"] is True and packet["timing_arms"] == 0
            and packet["order"] == config["orders"][0] and packet["diagnostic_arms"] == 8,
            "SETUP contract changed")
    for path, expected in packet["local_sha256"].items():
        require(digest(ROOT / path) == expected, f"changed controller input: {path}")
    require(digest(STAGE) == packet["stage_sha256"], "controller stage bytes changed")
    with tarfile.open(STAGE) as archive:
        require(archive.getnames() == sorted(packet["stage_members"]), "unexpected stage member")
        for member in archive.getmembers():
            require(member.isfile()
                    and storage.sha(archive.extractfile(member).read()) == packet["stage_members"][member.name],
                    f"bad stage member {member.name}")
    return packet, config


def report(archive, name):
    members = [member for member in archive.getmembers() if member.name == "./" + name]
    require(len(members) == 1 and members[0].isfile() and members[0].size < 20_000_000,
            f"missing or invalid build report {name}")
    return archive.extractfile(members[0]).read().decode()


def sha_manifest(text):
    pins = {}
    for line in text.splitlines():
        digest_, path = line.split("  ")
        require(re.fullmatch(r"[0-9a-f]{64}", digest_) and path not in pins, "invalid build digest manifest")
        pins[path] = digest_
    return pins


def build_identity(setup_dir, config):
    with tarfile.open(setup_dir / "build.tar.gz") as archive:
        require(report(archive, "exit.txt").strip() == "0", "private PostgreSQL build or bridge failed")
        require("passed" in report(archive, "bridge.txt"), "stock compatibility bridge failed")
        require(report(archive, "stock-readback.txt").count(": OK") == 5
                and report(archive, "stock-post-build.txt").count(": OK") == 5,
                "canonical PostgreSQL readback failed")
        require("invalid value for parameter" in report(archive, "stock-lz4-rejected.log"),
                "stock negative codec control failed")
        require("--with-lz4" in report(archive, "installed-abi.txt"), "private build lacks the codec flag")
        for library in ("postgres", "pg_stat_statements.so"):
            require(report(archive, f"{library}.stock.libraries.txt")
                    == report(archive, f"{library}.isolated.libraries.txt"),
                    f"{library} runtime dependency resolution changed")
        *host, ticks = report(archive, "host.txt").splitlines()
        require(len(host) == 3 and host[1:] == config["host"][1:] and int(ticks) == config["clock_ticks"],
                "frozen kernel, native disk or host clock tick rate changed")
        pins = sha_manifest(report(archive, "installed.sha256"))
        required = {"bin/postgres", "bin/initdb", "bin/pg_ctl", "bin/psql", "bin/pg_config",
                    "lib/pg_stat_statements.so", "share/extension/pg_stat_statements.control"}
        require(pins.keys() == required and pins["bin/postgres"] != config["postgres"]["postgres_sha256"],
                "build receipt does not pin the separate server and extension")
    return host, pins


def bound_config(packet, config, setup_dir):
    host, installed = build_identity(setup_dir, config)
    root = NATIVE / f"slow-lz4-isolated-{packet['stage_sha256'][:16]}"
    work, stage = root / "work", root / "stage"
    prefix = work / PG_NAME
    result = deepcopy(config)
    result.update(host=host, remote=str(stage), postgres={
        "prefix": str(prefix), "installed": installed, "source": SOURCES,
        "build_report_sha256": digest(setup_dir / "build.tar.gz"),
    })
    for lane in result["lanes"].values():
        engine = lane["engine"]
        spec = lane["inventory"]["engines"][engine]
        argv = spec["argv"]
        if engine in ("mesh", "oban"):
            argv[2:3] = [str(stage / "native-run"), str(prefix), str(work)]
            require(argv[5] == engine, "stock native launcher shape changed")
            scratch = work / "scratch"
        else:
            argv[2] = str(stage / "overlay.py")
            pg = argv.index("--pg-bin")
            require(argv[pg + 1] == str(STOCK / "bin"), "stock peer PostgreSQL path changed")
            argv[pg + 1] = str(prefix / "bin")
            scratch = work / ("river" if engine == "go" else "apalis")
            argv[argv.index("--scratch") + 1] = str(scratch)
        spec["postgres_storage"] = str(scratch)
        verified = {path: sha for path, sha in spec["verified_files"].items()
                    if not path.startswith(config["remote"] + "/")}
        verified.update({str(stage / name): packet["stage_members"][name]
                         for name in (*HOST_FILES, "storage.py", "slow-river.sql")})
        verified.update({str(STOCK / file): sha for file, sha in sha_manifest((HERE / "stock.sha256").read_text()).items()})
        verified.update({str(prefix / file): sha for file, sha in installed.items()})
        spec["verified_files"] = verified
        spec["evidence_globs"] = sorted(set(spec["evidence_globs"]) | {"codec-postgres.json"})
        matrix.check_inventory(lane["inventory"])
    return result


def score(directory, label, config, phase):
    lane = config["lanes"][label]
    engine, policy = lane["engine"], lane["policy"]
    score = original.score(directory, lane, config, phase)
    with tarfile.open(directory / f"{phase}-{engine}/raw.tar.gz") as archive:
        ledger = matrix.file_json(archive, "adapter/ledger.json")
        receipt = matrix.file_json(archive, "adapter/payload-storage.json")
        pg = matrix.file_json(archive, "adapter/codec-postgres.json")
    expected_settings = {
        "default_toast_compression": policy, "ssl": "on", "fsync": "on", "full_page_writes": "on",
        "synchronous_commit": "on", "track_commit_timestamp": "on", "max_connections": "68",
        "reserved_connections": "0", "superuser_reserved_connections": "0",
        "shared_preload_libraries": "pg_stat_statements",
    }
    require(pg == {"postgres": config["postgres"]["prefix"] + "/bin/postgres",
                   "postgres_sha256": config["postgres"]["installed"]["bin/postgres"],
                   "server_version": "170011", "settings": expected_settings},
            f"{label}: live PostgreSQL identity or settings changed")
    verify_security(engine, receipt["security"])
    score["attempt_id"] = read(directory / f"{phase}-{engine}/request.json")["attempt"]
    score["compression_counts"] = verify_codec(storage, engine, receipt["persisted"], ledger["jobs"], policy)
    score.update(write_landmarks(ledger, score["first_commit_us"]))
    return score


def verify_security(engine, receipt):
    limits = {"mesh": (4, 1), "oban": (66, 1), "go": (-1, -1), "rust": (-1, 1)}
    subject, producer = limits[engine]
    roles = {
        "parity_subject": {"login": True, "superuser": False, "connection_limit": subject},
        "parity_producer": {"login": True, "superuser": False, "connection_limit": producer},
    }
    if engine == "mesh":
        roles.update(
            compiled_caller={"login": True, "superuser": False, "connection_limit": 8},
            compiled_definer={"login": False, "superuser": False, "connection_limit": -1},
        )
    require(receipt == {"database_limit": 68, "observer_tls": True, "roles": roles, "codec_overrides": 0},
            f"{engine}: role limits, privileges, TLS, or codec overrides differ")


def setup_replay(packet, config):
    directory = OUT / "setup"
    bound = bound_config(packet, config, directory)
    require(read(directory / "config.json") == bound, "setup inventory differs from the native build receipt")
    scores = replay_block(directory / "00-slow", 0, bound, ["diagnostic"])
    return bound, scores


def replay_block(directory, group, config, phases):
    order = config["orders"][max(group - 1, 0)]
    require(read(directory / "order.json") == dict(group=group, order=order, phases=phases),
            "recorded profile rotation changed")
    scores = {f"{phase}-{label}": score(directory / f"{phase}-{label}", label, config, phase)
              for phase in phases for label in order}
    require(len({arm["attempt_id"] for arm in scores.values()}) == len(scores), "repeated native arm ID")
    require(read(directory / "reconciled.json") == scores, "controller and independent replay scores differ")
    return scores


def grant(kind, packet_path, seconds):
    id_ = read(packet_path)["id"]
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                        (QUEUE / "STATUS.md").read_text(), re.M)
    require(events and events[-1][:2] == (kind, id_), f"no current controller {kind} grant for {id_}")
    _, _, start, end = events[-1]
    now = datetime.now(timezone.utc)
    require(datetime.fromisoformat(start) <= now < datetime.fromisoformat(end) - timedelta(seconds=seconds),
            "insufficient exclusive grant time")
    require((QUEUE / f"{id_}.json").read_bytes() == packet_path.read_bytes(), "controller queue seal mismatch")
    return dict(id=id_, kind=kind, from_utc=start, until_utc=end, packet_sha256=digest(packet_path))


def dispatch(directory, config, phase_groups, packet_path, kind):
    remote = {}
    block = {"profile": "slow"}
    for label, lane in config["lanes"].items():
        grant(kind, packet_path, 90)
        remote[label] = matrix.REMOTE / f"slow-lz4-{uuid4().hex}-{label}"
        stage = directory / "inventory" / label
        stage.mkdir(parents=True)
        write(stage / "inventory.json", lane["inventory"])
        with (stage / "transfer.log").open("xb") as log:
            matrix.stage(SUITE / "worker.py", remote[label], log)
            matrix.stage(stage / "inventory.json", remote[label], log)
    for group, phases in phase_groups:
        run = directory / f"{group:02d}-slow"
        run.mkdir()
        order = config["orders"][max(group - 1, 0)]
        write(run / "order.json", dict(group=group, order=order, phases=phases))
        scores = {}
        for phase in phases:
            for label in order:
                grant(kind, packet_path, 90)
                arm = run / f"{phase}-{label}"
                arm.mkdir()
                matrix.dispatch_arm(arm, remote[label], block, phase, config["lanes"][label]["engine"])
                scores[f"{phase}-{label}"] = score(arm, label, config, phase)
                write(arm / "score.json", scores[f"{phase}-{label}"])
        write(run / "reconciled.json", scores)


def execute_setup(packet, config):
    grant("SETUP", SETUP, 300)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        admission = grant("SETUP", SETUP, 300)
        directory = OUT / "setup"
        directory.mkdir(parents=True)
        write(directory / "grant.json", admission)
        command = ["bash", "-c", (HERE / "native-build.sh").read_text(),
                   "slow-lz4-native-build", packet["stage_sha256"]]
        with STAGE.open("rb") as stage, (directory / "build.tar.gz").open("xb") as raw, \
                (directory / "build-transport.log").open("xb") as log:
            matrix.ssh(command, stdin=stage, stdout=raw, stderr=log, timeout=1200).check_returncode()
        config = bound_config(packet, config, directory)
        write(directory / "config.json", config)
        dispatch(directory, config, [(0, ["diagnostic"])], SETUP, "SETUP")
        matrix.ssh(["bash", "-c", original.HOST_IDLE], timeout=12).check_returncode()


def setup_release():
    path = QUEUE / f"{ID}.released.json"
    release = read(path)
    require(release["id"] == ID and release["exit_status"] == 0 and release["released_at"],
            "needs the controller's successful canonical SETUP release")
    require(read(OUT / "setup/grant.json")["packet_sha256"] == digest(SETUP),
            "SETUP did not bind the sealed request")
    return path


def seal_timing(packet, config):
    release = setup_release()
    bound, scores = setup_replay(packet, config)
    config_path = HERE / "matched-inventory.json"
    write(config_path, bound)
    command = f"python3 -B {shlex.quote(str(Path(__file__).resolve()))}"
    write(TIMING, {
        "id": ID + "-matched-timing", "kind": "TIMING", "requested_window_minutes": 45,
        "setup_release_sha256": digest(release), "setup_packet_sha256": digest(SETUP),
        "setup_raw": {label: row["raw_sha256"] for label, row in scores.items()},
        "inventory": str(config_path), "inventory_sha256": digest(config_path),
        "groups": list(range(1, 6)), "phases": ["warmup", "measured"], "arms": 80,
        "evidence_destination": str(OUT / "timing"), "timing_command": command + " timing",
        "offline_check_command": command + " check-timing", "replay_command": command + " replay-timing",
    })


def check_timing(packet, config):
    timing = read(TIMING)
    bound, scores = setup_replay(packet, config)
    path = HERE / "matched-inventory.json"
    require(timing["setup_release_sha256"] == digest(setup_release())
            and timing["setup_packet_sha256"] == digest(SETUP)
            and timing["setup_raw"] == {label: row["raw_sha256"] for label, row in scores.items()}
            and timing["inventory_sha256"] == digest(path) and read(path) == bound
            and timing["groups"] == list(range(1, 6)) and timing["phases"] == ["warmup", "measured"]
            and timing["arms"] == 80, "timing seal or qualification replay differs")
    return timing, bound


def execute_timing(timing, config):
    grant("TIMING", TIMING, 300)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        admission = grant("TIMING", TIMING, 300)
        directory = OUT / "timing"
        directory.mkdir()
        write(directory / "grant.json", admission)
        matrix.ssh(["bash", "-c", original.HOST_IDLE], timeout=12).check_returncode()
        dispatch(directory, config, [(group, timing["phases"]) for group in timing["groups"]], TIMING, "TIMING")
        matrix.ssh(["bash", "-c", original.HOST_IDLE], timeout=12).check_returncode()


def timing_replay(timing, config):
    release = read(QUEUE / f"{timing['id']}.released.json")
    require(release["id"] == timing["id"] and release["exit_status"] == 0 and release["released_at"]
            and read(OUT / "timing/grant.json")["packet_sha256"] == digest(TIMING),
            "needs the successful canonical TIMING release for this exact packet")
    rows = {group: replay_block(OUT / "timing" / f"{group:02d}-slow", group, config, timing["phases"])
            for group in timing["groups"]}
    require(len({arm["attempt_id"] for block in rows.values() for arm in block.values()}) == 80,
            "timing arms were reused across blocks")
    peers = ("river", "oban", "apalis")
    policies = ("pglz", "lz4")
    mesh_minus_peers = {
        policy: {peer: {clock: original.distribution([
            block[f"measured-bulk-{policy}"][clock] - block[f"measured-{peer}-{policy}"][clock]
            for block in rows.values()]) for clock in CLOCKS} for peer in peers} for policy in policies
    }
    within_engine = {}
    for engine in ("bulk", *peers):
        pglz, lz4 = f"measured-{engine}-pglz", f"measured-{engine}-lz4"
        sample = rows[1][pglz]
        metrics = [key for key, value in sample.items()
                   if key.endswith(("_ms", "_bytes", "_ms_per_job")) and type(value) in {int, float}
                   and all(type(block[label].get(key)) in {int, float}
                           for block in rows.values() for label in (pglz, lz4))]
        within_engine[engine] = {
            metric: original.distribution([block[pglz][metric] - block[lz4][metric] for block in rows.values()])
            for metric in sorted(metrics)
        }
    return {"series": "ordinary-slow-storage-transfer-isolated-pglz-v-lz4", "rows": rows,
            "paired_pglz_minus_lz4": within_engine, "within_block_mesh_minus_peer": mesh_minus_peers}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("seal-setup", "check", "setup", "replay-setup",
                                           "seal-timing", "check-timing", "timing", "replay-timing"))
    action = parser.parse_args().action
    if action == "seal-setup":
        seal_setup()
        return
    packet, config = check()
    if action == "check":
        print(digest(SETUP))
    elif action == "setup":
        execute_setup(packet, config)
    elif action == "replay-setup":
        _, scores = setup_replay(packet, config)
        print(json.dumps(scores, indent=2, sort_keys=True))
    elif action == "seal-timing":
        seal_timing(packet, config)
    else:
        timing, bound = check_timing(packet, config)
        if action == "check-timing":
            print(digest(TIMING))
        elif action == "timing":
            execute_timing(timing, bound)
        else:
            print(json.dumps(timing_replay(timing, bound), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
