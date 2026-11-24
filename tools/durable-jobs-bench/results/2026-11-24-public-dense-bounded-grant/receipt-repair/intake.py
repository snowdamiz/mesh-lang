"""Launch the sealed grant experiment with JSON-compatible controller receipts."""
from hashlib import sha256
from pathlib import Path
import sys


PACKET = Path(__file__).resolve().parents[1]
ORIGINAL = PACKET / "controller.py"
PIN = "2ec9dbb059688019c410a6c82e34d6800f57436f1a2e5d697678e7e1cd4695a9"
REQUEST = "cfc4c7221d086f0d00614ecd53f0222e2696abc0f5481ade6d5fc603a7ce7174"


def load(repaired=True):
    source = ORIGINAL.read_bytes()
    if sha256(source).hexdigest() != PIN:
        raise ValueError("bounded grant controller differs from its sealed source")
    old = b', range(start, end + 1))'
    if source.count(old) != 1:
        raise ValueError("expected one group argument at the controller boundary")
    if repaired:
        source = source.replace(old, b', list(range(start, end + 1)))')
    sys.path.insert(0, str(PACKET))
    driver = dict(__file__=str(ORIGINAL), __name__="bounded_grant")
    exec(compile(source, str(ORIGINAL), "exec"), driver)
    return driver


if __name__ == "__main__":
    if sys.argv[1:] not in (["intake", REQUEST], ["intake-peers", REQUEST], ["check"]):
        sys.exit("expected check or an intake with the original request digest")
    sys.exit(load()["main"](sys.argv[1:]))
