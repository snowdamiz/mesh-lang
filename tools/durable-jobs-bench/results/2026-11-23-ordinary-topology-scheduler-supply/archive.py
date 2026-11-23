"""Seal one local series with the unmodified redacted evidence and dispatch log."""

import argparse
import gzip
from pathlib import Path
import re
import tarfile


PRIVATE = re.compile(rb"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----|SCRAM-SHA-256\$|"
                     rb"postgres(?:ql)?://[^:\s/]+:(?!<redacted>)[^\s@]+@", re.I)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("series", type=Path)
    parser.add_argument("archive", type=Path)
    args = parser.parse_args()
    series = args.series
    paths = sorted(path for path in series.rglob("*") if path.is_file())
    paths.append(series.with_suffix(".log"))
    for path in paths:
        if path.is_symlink() or path.suffix not in {".json", ".log"} or PRIVATE.search(path.read_bytes()):
            raise ValueError(f"unsupported or private evidence member: {path.name}")
    with args.archive.open("xb") as stream, gzip.GzipFile(
            filename="", fileobj=stream, mode="wb", mtime=0) as zipped, tarfile.open(
                fileobj=zipped, mode="w|") as archive:
        for path in paths:
            entry = tarfile.TarInfo(path.relative_to(series.parent).as_posix())
            entry.size = path.stat().st_size
            entry.mode = 0o644
            with path.open("rb") as source:
                archive.addfile(entry, source)
