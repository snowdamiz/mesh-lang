"""Build the deterministic source archive and offline handoff manifest."""
import gzip
from pathlib import Path
import shlex
import tarfile

import drive as d

SOURCE = d.WORK / "source"


def normalize(info):
    info.uid = info.gid = info.mtime = 0
    info.uname = info.gname = ""
    info.pax_headers = {}
    if info.name.startswith("source/target") or "__pycache__" in info.name:
        return None
    return info


def command(*args):
    return shlex.join(("python3", "-B", str(d.HERE / "drive.py"), *args))


def main():
    assert not d.PACKET.exists() and not d.ARCHIVE.exists()
    with d.ARCHIVE.open("xb") as out, gzip.GzipFile(filename="", mode="wb", fileobj=out, mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            archive.add(SOURCE, arcname="source", filter=normalize)
    paths = list(d.HERE.iterdir())
    paths += [d.PIN, d.WINNER / "controller.py", d.WINNER / "score.py", d.base["SOURCE"],
              d.SUITE / "worker.py", d.SUITE / "run.py", d.SUITE / "interleave.py"]
    raw = str(d.OUT / "setup/raw.tar.gz")
    release = str(d.OUT / "setup/controller-release.json")
    d.run.write_json(d.PACKET, dict(id=d.NAME, ready=True, after_existing_bookings=True,
        controller="native-controller-complete-post-wal-queue", native_host=d.base["logical"].HOST,
        cpus=[0, 1], source_sha=d.grouped.BASE, source_archive_sha256=d.run.digest(d.ARCHIVE),
        winner_elf_sha256=d.base["ELF"], input_sha256=d.base["transfer"].INPUT, d388_sha256=d.SQL,
        evidence_destination=str(d.OUT), requested_window_minutes=16,
        setup_grant_id=d.SETUP, setup_spec_command=command("setup-spec"),
        verification_command=command("bind", raw, release), offline_check_command=command("check"),
        schedule=d.PRIMARY, timing_window_minutes={d.SMOKE: 5, d.PILOT: 16, d.PEERS_ID: 26},
        timing_commands={key: command("dispatch", key, raw, release) for key in d.PRIMARY},
        replay_commands={key: command("replay", key, raw, release) for key in d.PRIMARY},
        local_sha256={str(path.relative_to(d.ROOT)): d.run.digest(path) for path in sorted(paths) if path.is_file()}))


if __name__ == "__main__":
    main()
