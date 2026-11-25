#!/usr/bin/env python3
"""Bind a separate controller request to the failed stock-formatting recovery."""
from pathlib import Path
import shlex
import sys
import tarfile

RECOVERY = Path(__file__).resolve().parent
BASE = RECOVERY.parent
sys.path.insert(0, str(BASE))
import packet
from check_stock_flags import verify as check_flags

FIRST = BASE / "evidence/native/setup"
FIRST_SEAL = "c977c5940910a7952697338d418c054970291ea1d16d8f9cb3a53050351818a8"
FIRST_RELEASE = FIRST / "controller-release.json"
HELD = BASE / "recovery/controller-packet.json"
HELD_SEAL = "8a8479a3cf0c4250a9ad78457eae5c34feb9c26ceb33925a9b3ab76810abaa68"
original_build_identity = packet.build_identity


def stage_files():
    files = {name: BASE / name for name in packet.HOST_FILES}
    files.update({name: packet.OLD / "replay" / name for name in ("storage.py", "slow-river.sql")})
    files.update({name: packet.LANE / "sources" / name for name in packet.SOURCES})
    files["check-stock-flags.py"] = RECOVERY / "check_stock_flags.py"
    files["build-pg.sh"] = RECOVERY / "build-pg.sh"
    files["bridge.sh"] = RECOVERY / "bridge.sh"
    return files


def bind_recovery():
    packet.HERE = RECOVERY
    packet.__file__ = __file__
    packet.ID += "-recovery-2"
    packet.SETUP = RECOVERY / "controller-packet.json"
    packet.TIMING = RECOVERY / "timing-packet.json"
    packet.OUT = BASE / "evidence/native/recovery-2"
    packet.STAGE = packet.LANE / "native-stage-recovery-2.tar"
    packet.stage_files = stage_files
    packet.build_identity = build_identity


def build_identity(setup_dir, config):
    host, pins = original_build_identity(setup_dir, config)
    with tarfile.open(setup_dir / "build.tar.gz") as archive:
        extensions = packet.sha_manifest(packet.report(archive, "apalis-extensions.sha256"))
        for extension in ("pgcrypto", "hstore"):
            library = f"{extension}.so"
            packet.require(f"lib/{library}" in extensions and f"share/extension/{extension}.control" in extensions,
                           f"Apalis extension {extension} was not pinned")
            packet.require(packet.report(archive, f"{library}.stock.libraries.txt")
                           == packet.report(archive, f"{library}.isolated.libraries.txt"),
                           f"{extension} native ABI dependencies differ")
        for path in extensions:
            packet.require(path in ("lib/pgcrypto.so", "lib/hstore.so")
                           or any(path.startswith(f"share/extension/{name}") and path.endswith((".sql", ".control"))
                                  for name in ("pgcrypto", "hstore")), "unknown extension manifest path")
        packet.require("pg_stat_statements, pgcrypto, and hstore" in packet.report(archive, "bridge.txt"),
                       "the stock compatibility bridge omitted an Apalis extension")
    return host, pins | extensions


def verify_trigger():
    packet.require(packet.digest(HELD) == HELD_SEAL, "withdrawn recovery seal changed")
    first_request = BASE / "controller-packet.json"
    packet.require(packet.digest(first_request) == FIRST_SEAL, "original setup seal changed")
    release = packet.read(FIRST_RELEASE)
    packet.require(release["id"] == BASE.name and release["exit_status"] == 1 and release["released_at"],
                   "missing the first controller failure and release")
    flags = (FIRST / "stock-pg-readback.log").read_text().splitlines()[1].removeprefix("pg_config:")
    check_flags(str(packet.STOCK), flags)
    for corrupted in (flags + " --with-lz4", "--with-openssl", flags.replace("--with-openssl", "--without-icu")):
        try:
            check_flags(str(packet.STOCK), corrupted)
        except ValueError:
            continue
        raise ValueError("stock compiler fingerprint accepted modified flags")


def seal():
    verify_trigger()
    cfg = packet.original_config()
    packet.build_stage()
    prior = packet.read(BASE / "controller-packet.json")
    command = f"python3 -B {shlex.quote(str(RECOVERY / 'recover.py'))}"
    pins = dict(prior["local_sha256"])
    for path in (RECOVERY / "recover.py", RECOVERY / "native-build.sh",
                 RECOVERY / "check_stock_flags.py", RECOVERY / "build-pg.sh", RECOVERY / "bridge.sh",
                 HELD, FIRST_RELEASE, FIRST / "stock-pg-readback.log",
                 FIRST / "build.tar.gz", BASE / "controller-packet.json"):
        pins[str(path.relative_to(packet.ROOT))] = packet.digest(path)
    packet.write(packet.SETUP, dict(
        prior, id=packet.ID, stage=str(packet.STAGE), stage_sha256=packet.digest(packet.STAGE),
        stage_members={name: packet.digest(path) for name, path in stage_files().items()},
        evidence_destination=str(packet.OUT / "setup"), local_sha256=pins,
        recovery_of=FIRST_SEAL, withdrawn_setup_seal=HELD_SEAL, original_release_sha256=packet.digest(FIRST_RELEASE),
        recovery_scope="Use exact parsed configure flags from the authenticated stock readback. "
                       "Build and bind PostgreSQL 17.11 contrib pgcrypto and hstore for pinned Apalis migrations. " 
                       "Verify extension ABI and use a shorter private compatibility socket path.",
        offline_check_command=command + " check", setup_command=command + " setup",
        verify_setup_command=command + " replay-setup", order=cfg["orders"][0],
    ))


if __name__ == "__main__":
    bind_recovery()
    if sys.argv[1:] == ["seal-setup"]:
        seal()
    else:
        verify_trigger()
        packet.main()
