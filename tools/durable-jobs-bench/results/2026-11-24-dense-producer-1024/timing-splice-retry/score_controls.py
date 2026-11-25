"""Exercise the real scorer with an archived stock ledger and proof receipts."""
from io import BytesIO
import json
from pathlib import Path
import shutil
import tarfile
from tempfile import TemporaryDirectory


def run_controls(original, config, stages, setup_raw):
    run = original.run
    stock = original.stock.OUTPUT
    ref = run.read_json(stock / "final-replay.json")["blocks"]["1"]
    source, = (stock / "blocks" / ref["attempt"] / "measured-hot").glob("measured-mesh")
    with tarfile.open(source / "raw.tar.gz") as archive:
        base = {m.name: archive.extractfile(m).read() for m in archive.getmembers() if m.isfile()}
    require = original.require
    require(original.dense.digest((source / "raw.tar.gz").read_bytes())
            == ref["scores"]["measured-hot"]["raw_sha256"], "stock scorer input changed")
    remote = run.REMOTE / "offline-splice-scorer"
    origins, = json.loads(base["adapter/ledger.json"])["transactions"]
    with tarfile.open(setup_raw) as archive:
        get = original.dense.archive_files(archive)
        proof = {w: json.loads(get(f"proof-owner/proof/canonical-{w}-durable.json")) for w in ("128", "1024")}

    def exercise(folder, label, width, fault=None):
        folder.mkdir()
        shutil.copyfile(source / "request.json", folder / "request.json")
        _, inventory = stages(config, label, remote)
        data = dict(base)
        host = json.loads(data["host.json"])
        host["verified_files"] = inventory["engines"]["mesh"]["verified_files"]
        launches = json.loads(data["adapter/process-identity.json"])
        broker = launches[0]
        executable = config["private_stage"] + "/broker/mesh"
        broker["argv"][0] = executable
        broker["executable_sha256"] = config["broker"]["files"]["mesh"] if fault != "broker" else "a" * 64
        if broker["observed_executable"] is not None:
            broker["observed_executable"] = executable
        receipts = []
        for row in proof[width]:
            ident = origins["enqueued_ids"][row["key"]]
            receipt = dict(row["receipt"], id=ident, effect_key=ident + "/1")
            receipts.append(dict(row, receipt=receipt))
        if fault == "missing":
            receipts.pop()
        wide = width == "1024"
        writer = dict(enabled=True, bulk=0 if wide else 8, scalar=0,
                      statements={"wide": [1, 1024]} if wide else {"narrow": [8, 1024]})
        for name, value in (("host.json", host), ("adapter/process-identity.json", launches),
                            ("adapter/producer-receipts.json", receipts), ("adapter/producer-writer.json", writer)):
            data[name] = json.dumps(value).encode()
        with tarfile.open(folder / "raw.tar.gz", "w:gz") as archive:
            for name, content in data.items():
                member = tarfile.TarInfo(name)
                member.size = len(content)
                archive.addfile(member, BytesIO(content))
        return original.score_arm(folder, "measured", label, config, remote)["completed"]

    results = {}
    with TemporaryDirectory(prefix="splice-score-", dir=original.ROOT / "target") as scratch:
        root = Path(scratch)
        for label, width in (("128-a", "128"), ("1024", "1024")):
            require(exercise(root / label, label, width) == 1024, "control lost canonical completions")
            results[label] = "accepted"
        for case, width, fault, reason in (
            ("eight-call-wide", "128", None, "actual writer calls or returned row count changed"),
            ("missing-receipt", "1024", "missing", "durable receipts differ"),
            ("wrong-broker", "1024", "broker", "direct process identity differs"),
        ):
            try:
                exercise(root / case, "1024", width, fault)
            except ValueError as exc:
                require(str(exc).startswith(reason), f"{case}: wrong scorer rejection: {exc}")
                results[case] = str(exc)
            else:
                raise ValueError(f"{case} incorrectly received a score")
    return dict(kind="offline synthetic controls using archived data", results=results)
