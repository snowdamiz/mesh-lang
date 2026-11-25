"""Recover the source and adapter used with the exact native FILE broker."""

import difflib
import hashlib
import io
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
PRODUCT = HERE.parent / "2026-11-24-trusted-jobs-product-composition"
LARGE = HERE.parent / "2026-11-24-public-large-file-session"
RECOVERED = LARGE / "resume/evidence/native/setup-export-recovered"


def member(archive, path):
    return archive.extractfile(path).read()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def inputs():
    with tarfile.open(LARGE / "resume/stage.tar.gz") as outer:
        with tarfile.open(fileobj=io.BytesIO(member(outer, "parked-stage.tar.gz"))) as source:
            base = member(source, "original-main.mpl")
            file = member(source, "broker-main.mpl")
            adapter = {name: member(source, "adapter/" + name)
                       for name in ("backend.py", "identity.py")}
    with tarfile.open(RECOVERED / "raw.tar.gz") as raw:
        pins = {Path(path).name: digest for digest, path in
                (line.split() for line in member(raw, "./adapter.sha256").decode().splitlines())}
        assert all(sha(data) == pins[name] for name, data in adapter.items())
    root = "source/tools/durable-jobs-bench/results/" + PRODUCT.name
    with tarfile.open(PRODUCT / "source.tar.gz") as stock:
        assert member(stock, root + "/fixture/main.mpl") == base
        grants = member(stock, "source/scripts/fixtures/jobs-role-grants.sql")
    diff = list(difflib.ndiff(base.decode().splitlines(), file.decode().splitlines()))
    added = [line[2:].strip() for line in diff if line.startswith("+ ")]
    assert not any(line.startswith("- ") for line in diff)
    assert len(added) == 14 and 'else if verb == "enqueue_file" do' in added
    assert "let receipt = transact_batch(conn, input)?" in added
    assert sum("File.read(" in line for line in file.decode().splitlines()) == 1
    return adapter, dict(stock_source=sha(base), broker_source=sha(file), role_grants=sha(grants))


if __name__ == "__main__":
    import sys

    adapter, _ = inputs()
    for filename, data in adapter.items():
        path = HERE / "stage" / filename
        if sys.argv[1:] == ["write"]:
            path.write_bytes(data)
        else:
            assert path.read_bytes() == data, path
