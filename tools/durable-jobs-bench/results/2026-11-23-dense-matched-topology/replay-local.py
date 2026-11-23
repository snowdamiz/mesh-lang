"""Reconcile the archived local two-runner peer and producer smokes."""

import runpy
import tarfile
from tempfile import TemporaryDirectory
from pathlib import Path

from matrix import PACKET, WORK, require, run


def main():
    WORK.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(prefix="archive-replay-", dir=WORK) as temp:
        path = Path(temp)
        with tarfile.open(PACKET / "evidence/local/local-smokes.tar.gz") as archive:
            archive.extractall(path, filter="data")
        for driver, receipt in (("peer-offline.py", "offline-peers.json"), ("bulk.py", "offline-bulk.json")):
            replay = runpy.run_path(str(PACKET / driver))["local"]
            replay.__globals__["WORK"] = path
            require(replay() == run.read_json(PACKET / receipt), f"{driver} disagrees with {receipt}")
            print(f"replayed {receipt}")


if __name__ == "__main__":
    main()
