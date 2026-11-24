"""Exercise binding and inventories with mock archived SETUP data and real peer pins."""
import json
from io import BytesIO
from pathlib import Path
import sys
import tarfile
from tempfile import TemporaryDirectory

import transfer as t

root = Path(sys.argv[1]).resolve(strict=True)
evidence = t.run.read_json(t.RESULTS / t.PRODUCT / "local-evidence.json")
manifest, witness = evidence["local_fixture"], evidence["teardown_witness"]
# Synthetic platform and compiler identifiers exercise the portable schema.
# The binder test does not claim that the local images are native ELFs.
for row in (manifest, witness):
    row.update(system="Linux", architecture="x86_64", compiler_sha256=t.COMPILER_SHA)
files = {
    "mesh.json": json.dumps(manifest).encode(),
    "witness.json": json.dumps(witness).encode(),
    "elf.txt": b"Class: ELF64\nMachine: Advanced Micro Devices X86-64\n",
    "toolchain.txt": b"rustc 1.99.0\n21.1.8\n17.11\n90ec1c24-08c8-4bc1-aa28-79b6db1bd13a\n6.8.0-149-generic\n8:0\n",
}
files.update({f"semantic-root/semantic/{key}/result.json": json.dumps(val).encode()
              for key, val in evidence["checks"].items()})

def write_tar(path, content):
    with tarfile.open(path, "w:gz") as raw:
        for key, data in content.items():
            info = tarfile.TarInfo("./" + key)
            info.size = len(data)
            raw.addfile(info, BytesIO(data))

def refuse(release, raw):
    try:
        t.setup_binding(release, raw)
    except (ValueError, KeyError):
        return
    raise AssertionError("binder accepted invalid SETUP")

with TemporaryDirectory(dir=root, prefix="offline-control-") as work:
    work = Path(work)
    release = work / "release.json"
    release.write_text(json.dumps(dict(id=t.PRODUCT, exit_status=0)))
    raw = work / "setup.tar.gz"
    write_tar(raw, files)
    bound = t.setup_binding(release, raw)
    config = t.configurations(bound)
    specs = {}
    for lane in config["lanes"]:
        _, inv = t.stages(config, lane, t.run.REMOTE / "offline-dense-product")
        specs[lane] = inv["engines"][config["lanes"][lane]["engine"]]
    assert specs["checked-a"] == specs["checked-b"]
    paired = dict(specs["checked-a"], argv=[
        arg.replace("PRODUCT_COMPOSITION=product-group", "PRODUCT_COMPOSITION=product-d2")
        for arg in specs["checked-a"]["argv"]])
    assert paired == specs["hot"]
    for peer in t.PEERS:
        prior = t.run.read_json(t.R2_ROOT / "evidence/native-import-retry/staging" / peer / "inventory.json")
        actual = specs[peer]
        expected = prior["engines"][config["lanes"][peer]["engine"]]
        # Only the controller-owned worker.py lives under the new remote workspace.
        assert {k:v for k,v in actual.items() if k != "verified_files"} == {
            k:v for k,v in expected.items() if k != "verified_files"}
        assert {k:v for k,v in actual["verified_files"].items() if not k.endswith("/worker.py")} == {
            k:v for k,v in expected["verified_files"].items() if not k.endswith("/worker.py")}
    expected_setup = t.run.read_json(t.RESULTS / t.PRODUCT / "controller-packet.json")["setup_output"]["artifact"]
    assert expected_setup in specs["hot"]["verified_files"]
    assert t.setup_binding(release, raw) == bound
    release.write_text(json.dumps(dict(id=t.PRODUCT, exit_status=1)))
    refuse(release, raw)
    release.write_text(json.dumps(dict(id=t.PRODUCT, exit_status=0)))
    missing = {key: val for key,val in files.items() if "ambiguous-application-write" not in key}
    write_tar(work / "missing.tar.gz", missing)
    refuse(release, work / "missing.tar.gz")

history = t.run.read_json(root / "original-dense-replay.json")
assert [
    int(group) for group, block in sorted(history["blocks"].items(), key=lambda row:int(row[0]))
    if t.contrast({"checked-a": block["raw_scores"]["measured-d2-off-a"],
                   "checked-b": block["raw_scores"]["measured-d2-off-b"],
                   "hot": block["raw_scores"]["measured-d2-on"]}, 0.05)["qualifies"]
] == [1, 2, 4, 6]
print("Portable binder and refusal controls passed. Same-ELF argv delta, exact peers, and historical gate verified.")
