"""Link a session-only change against the sealed public fixture and toolchain."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT / "tools/durable-jobs-bench/native/direct-producer"))
from prepare import producer_source, replace_once

ELF = "af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355"
REVISION = "b3557273d72be3802bf189b284e7caaf5f3caa0f"
ARCHIVE = "623c46b92cc5f94eb19d455e36e6fca1e1b800231441a6de480e2c35aa091b02"
MANIFESTS = {
    "Linux": "979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55",
    "Darwin": None,
}
INLINE = '    let receipt = transact_batch(conn, Json.object_get(request, "batch")?)?'
FILE_BODY = "    println(receipt)"
TRACE = '  let batch = "[" <> String.join(specs, ",") <> "]"'
TRACE_LINE = '''  let frame = Bytes.from_utf8(batch)
  IO.eprintln("SPEC " <> "#{size} #{Bytes.length(frame)} " <> Bytes.to_hex(Crypto.sha256(frame)))'''


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def broker_source(main, trace=False):
    # Reuse the older broker's directory check, request-ID check and read_batch
    # path. Both verbs then call the exact same transact_batch on conn.
    previous = producer_source(main)
    start = previous.index('    let path = Json.as_string(Json.object_get(request, "batch")?)?')
    end = previous.index(FILE_BODY, start) + len(FILE_BODY)
    file_branch = previous[start:end]
    output = replace_once(main, f'{INLINE}\n{FILE_BODY}\n  else if verb == "query" do',
                          f'{INLINE}\n{FILE_BODY}\n  else if verb == "enqueue_file" do\n'
                          f'{file_branch}\n  else if verb == "query" do')
    if trace:
        output = replace_once(output, TRACE, TRACE + "\n" + TRACE_LINE)
    return output


def build(stock, compiler, runtime, out, trace=False):
    stock_manifest = stock / "mesh.json"
    pins = json.loads(stock_manifest.read_text())
    assert (pins["version"], pins["source_sha256"]) == (REVISION, ARCHIVE)
    expected = MANIFESTS[pins["system"]]
    if expected:
        assert sha(stock_manifest) == expected and pins["files"]["mesh"] == ELF
    else:
        assert pins["files"]["main.mpl"] == "1273ba6b01002d4c0a11a83cf827f7f83b412ff6d7c078cbfe8f21d77e8267e0"
    assert sha(compiler) == pins["compiler_sha256"] and sha(runtime) == pins["runtime_sha256"]
    for name, digest in pins["files"].items():
        assert sha(stock / name) == digest, name
    out.mkdir()
    for name in pins["files"].keys() - {"mesh", "main.mpl"}:
        shutil.copy2(stock / name, out / name)
    (out / "main.mpl").write_text(broker_source((stock / "main.mpl").read_text(), trace))
    with (out / "link.log").open("xb") as log:
        subprocess.run([compiler, "build", out, "--opt-level", "2", "--output", out / "broker"],
                       env=dict(os.environ, MESH_RT_LIB_PATH=str(runtime)), stdout=log,
                       stderr=subprocess.STDOUT, check=True)
    result = dict(stock_manifest=sha(stock_manifest), runner=pins["files"]["mesh"],
                  revision=REVISION, source_archive=ARCHIVE, compiler=sha(compiler),
                  runtime=sha(runtime), system=pins["system"], trace=trace,
                  files={p.name: sha(p) for p in sorted(out.iterdir()) if p.name != "link.log"})
    (out / "broker.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    trace = sys.argv[-1] == "--trace"
    build(*(Path(value).resolve() for value in sys.argv[1:5]), trace=trace)
