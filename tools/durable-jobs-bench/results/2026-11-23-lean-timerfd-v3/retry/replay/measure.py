"""Compare sealed controls and the opt-in worker on the same private relay."""
from hashlib import file_digest
import json
import os
from pathlib import Path
import resource
import sys

from component import WAVES, batch

STOCK_SHA = "3dc20c730bd269da86c9c98f225d5f85bdc0dd3d3a31908e0eac90aa50d8dfd2"
LEAN_SHA = "d863391a9f2b1b1ff7caedb8cd76008d04be7f06561f1bc8686cf8ce944d42e1"
ORDERS = (
    ("lean-a", "timerfd", "stock", "lean-b"),
    ("lean-b", "stock", "timerfd", "lean-a"),
    ("stock", "lean-b", "lean-a", "timerfd"),
    ("timerfd", "lean-a", "lean-b", "stock"),
    ("lean-a", "stock", "timerfd", "lean-b"),
)


def digest(path):
    with path.open("rb") as data:
        return file_digest(data, "sha256").hexdigest()


def run(stock, lean, out, output):
    assert os.uname().machine == "x86_64" and os.getuid() != 0
    assert sorted(os.sched_getaffinity(0)) == [0, 1]
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    closure = json.loads((out / "closure.json").read_text())
    fingerprint = closure["fingerprint"]
    assert fingerprint == "e03da7787d8c4083:Main__handle"
    images = {"stock": stock, "lean-a": lean, "lean-b": lean, "timerfd": out / "mesh-lean-worker"}
    pins = {"stock": STOCK_SHA, "lean-a": LEAN_SHA, "lean-b": LEAN_SHA, "timerfd": closure["worker_sha256"]}
    assert all(digest(path) == pins[label] for label, path in images.items())
    with output.open("x") as log:
        def retain(**row):
            print(json.dumps(row), file=log, flush=True)

        retain(kind="system", boot=Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
               hashes=pins, affinity=sorted(os.sched_getaffinity(0)), uname=list(os.uname()),
               load=os.getloadavg(), orders=ORDERS)
        for arm in ("stock", "lean-a", "timerfd"):
            retain(kind="warmup", arm=arm, **batch(images[arm], fingerprint, f"{arm}-warmup", 1))
        for i, order in enumerate(ORDERS, 1):
            for arm in order:
                retain(kind="sample", rotation=i, arm=arm, load=os.getloadavg(),
                       **batch(images[arm], fingerprint, f"r{i}-{arm}", WAVES))
        retain(kind="finish", load=os.getloadavg())


if __name__ == "__main__":
    run(*(Path(arg) for arg in sys.argv[1:]))
