"""Derive late-escape controls from byte-exact archived FILE frames."""
import hashlib
import json
from pathlib import Path
import sys

home = Path(sys.argv[1])
old = json.loads((home / "large-capture.json").read_text())["frames"]
public = json.loads((home / "public-file-capture.json").read_text())["frames"]
assert old == public and len(old) == 64
out = home / "frames" / "escaped"
out.mkdir()
pins = []
for i, expected in enumerate(old):
    raw = (home / "frames" / "large" / str(i)).read_bytes()
    assert len(raw) == expected["bytes"] and hashlib.sha256(raw).hexdigest() == expected["sha256"]
    spec, = json.loads(raw)
    payload = spec["args"]["payload"].encode("utf8")
    assert len(payload) == 524288 and len(payload) * 2 >= len(raw)
    assert not set(payload) & (set(range(32)) | {34, 92})
    marker = b'"payload":"'
    start = raw.index(marker) + len(marker)
    assert raw[start:start + len(payload)] == payload
    stop = start + len(payload) - 1
    escaped = raw[:stop] + b"\\u001f" + raw[stop+1:]
    altered, = json.loads(escaped)
    assert altered["args"]["payload"].encode("utf8") == payload[:-1] + b"\x1f"
    (out / str(i)).write_bytes(escaped)
    pins.append({"bytes":len(escaped), "sha256":hashlib.sha256(escaped).hexdigest()})
(home / "escaped-capture.json").write_text(json.dumps({"frames":pins}, indent=2, sort_keys=True) + "\n")
