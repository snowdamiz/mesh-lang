"""Package the exact Git build inputs and the accompanying Jobs proof overlay."""

import gzip
from pathlib import Path
import subprocess
import tarfile
import sys

REVISION = "e4156740c7409156409cd2f792f4875153d71040"
INPUTS = (
    ".cargo", "Cargo.toml", "Cargo.lock", "compiler", "tools/skill/mesh",
    "proof/autonomous-gates", "scripts/fixtures/jobs-role-grants.sql",
    "scripts/fixtures/jobs-set-based-claim-grant.sql",
)


def pack(output):
    packet = Path(__file__).resolve().parent
    root = packet.parents[3]
    command = ["git", "archive", "--format=tar", "--prefix=source/", REVISION, *INPUTS]
    with (output.open("xb") as file, gzip.GzipFile(filename="", mode="wb", fileobj=file, mtime=0) as compressed,
          tarfile.open(fileobj=compressed, mode="w|", format=tarfile.PAX_FORMAT) as archive,
          subprocess.Popen(command, cwd=root, stdout=subprocess.PIPE) as git):
        with tarfile.open(fileobj=git.stdout, mode="r|") as source:
            for entry in source:
                assert entry.isdir() or entry.isfile(), entry.name
                archive.addfile(entry, source.extractfile(entry) if entry.isfile() else None)
        assert git.wait() == 0
        for path in sorted((packet / "overlay").rglob("*")):
            assert path.is_file() or path.is_dir()
            assert path.suffix not in (".pyc",)
            entry = archive.gettarinfo(str(path), str(path.relative_to(packet)))
            entry.uid, entry.gid, entry.uname, entry.gname, entry.mtime = 0, 0, "", "", 0
            entry.mode = 0o644 if entry.isfile() else 0o755
            if entry.isfile():
                with path.open("rb") as stream:
                    archive.addfile(entry, stream)
            else:
                archive.addfile(entry)


if __name__ == "__main__":
    pack(Path(sys.argv[1]))
