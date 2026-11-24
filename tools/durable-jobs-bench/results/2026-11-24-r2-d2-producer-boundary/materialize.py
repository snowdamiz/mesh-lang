"""Apply and verify the benchmark-only probes against frozen input bytes."""
from hashlib import sha256
import json
from pathlib import Path
import sys


def replace(source, anchor, replacement):
    if source.count(anchor) != 1:
        raise ValueError(f"frozen anchor changed: {anchor}")
    return source.replace(anchor, replacement)


def runtime(directory, probe):
    edits = {
        "lib.rs": [
            ("pub mod io;", "pub mod io;\nmod r2_producer_probe;"),
        ],
        "io.rs": [
            ("            let s = mesh_str(&input);",
             "            crate::r2_producer_probe::line(&input);\n            let s = mesh_str(&input);"),
        ],
        "string.rs": [
            ('    unsafe { write_stdout((*s).as_str(), "\\n") }',
             '    crate::r2_producer_probe::publish();\n    unsafe { write_stdout((*s).as_str(), "\\n") }'),
        ],
        "db/pg.rs": [
            ('    send(conn, what, message)?;\n    let mut count = 0;',
             '    let mut probe = crate::r2_producer_probe::begin(message);\n'
             '    send(conn, what, message)?;\n'
             '    if let Some(probe) = &probe { probe.sent(); }\n    let mut count = 0;'),
            ('            .map_err(|error| format!("read {what}: {error}"))?;\n        match tag {',
             '            .map_err(|error| format!("read {what}: {error}"))?;\n'
             '        if let Some(probe) = &mut probe { probe.row(tag); }\n        match tag {'),
            ('                conn.txn_status = *status;\n                return failure.map_or(Ok(count), Err);',
             '                conn.txn_status = *status;\n'
             '                if let Some(probe) = &probe { probe.ready(); }\n'
             '                return failure.map_or(Ok(count), Err);'),
        ],
    }
    pins = {}
    for rel, replacements in edits.items():
        path = directory / rel
        stock = path.read_text()
        body = stock
        for anchor, replacement in replacements:
            body = replace(body, anchor, replacement)
        path.write_text(body)
        pins[rel] = [sha256(item.encode()).hexdigest() for item in (stock, body)]
    path = directory / "r2_producer_probe.rs"
    with path.open("xb") as stream:
        stream.write(probe)
    pins[path.name] = [None, sha256(probe).hexdigest()]
    return pins


if __name__ == "__main__":
    tree, probe = map(Path, sys.argv[1:])
    print(json.dumps(runtime(tree / "compiler/mesh-rt/src", probe.read_bytes()), sort_keys=True, indent=2))
