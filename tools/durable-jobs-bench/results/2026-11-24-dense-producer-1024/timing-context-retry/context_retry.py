"""Reuse the sealed timing coordinator with a second immutable adapter sibling."""
import json
from pathlib import Path
import sys
import tarfile

HERE = Path(__file__).resolve().parent
FIRST = HERE.parent / "timing-splice-retry"
sys.path.insert(0, str(FIRST))
import retry

run, require = retry.run, retry.require
PACKET = HERE / "intake.json"
EVIDENCE = HERE / "evidence"


def bind():
    prior, initial, broker = retry.bind()
    request = run.read_json(PACKET)
    for name, digest in request["pins"].items():
        require(run.digest(retry.ROOT / name) == digest, f"context retry pin changed: {name}")
    folder = FIRST / "evidence/pilot"
    released = run.read_json(folder / "controller-release.json")
    previous = run.read_json(folder / "replay.json")
    require(released["id"] == retry.original.ID + "-pilot-splice-retry"
            and released["exit_status"] == 1 and released["released_at"]
            and not previous["qualifies"] and previous["missing"] == [1, 2] and len(previous["raw"]) == 12,
            "the preceding pilot did not release as rejected")
    for name, expected in previous["raw"].items():
        path = folder / name
        require(run.digest(path) == expected, "preceding raw archive changed")
        with tarfile.open(path) as raw:
            failure = run.file_json(raw, "adapter/failure.json")
            require(run.file_json(raw, "status.json")["exit_code"] == 1
                    and failure["type"] == "TypeError"
                    and failure["message"] == "'_GeneratorContextManager' object is not an iterator",
                    "preceding arm failed outside broker context entry")
    return request, initial, broker


def stage_request(mode):
    path = HERE / (mode + ".json")
    data = run.read_json(path)
    require(data["id"] == retry.original.ID + f"-{mode}-context-retry"
            and data["intake_sha256"] == run.digest(PACKET) and data["groups"] == [1, 2],
            "context grant request changed")
    return data, path


def sibling(config):
    return Path(config["private_stage"]) / "packet-timing-context"


def check_preflight(path, config, asset_checks):
    log = path.read_text()
    local = asset_checks == "archived-digests"
    if local:
        require("packet-timing-splice/adapter.py" in log
                and "TypeError: '_GeneratorContextManager' object is not an iterator" in log,
                "preceding context failure was not reproduced")
    rows = [json.loads(line) for line in log.splitlines() if line.startswith("{")]
    require([row["width"] for row in rows] == ["128", "1024"], "broker context did not run for both widths")
    for row in rows:
        require(row == dict(adapter=str(sibling(config) / "adapter.py"),
                            adapter_sha256=run.digest(HERE / "adapter.py"), harness=str(retry.ARM),
                            harness_sha256=retry.ARM_SHA, uid=65534, gid=65534, cpus=[0, 1],
                            width=row["width"], no_jobs_submitted=True,
                            context="synthetic-broker" if local else "native-broker-tls"),
                "broker context proof differs")
    return rows


def main():
    validated = bind()
    retry.HERE, retry.EVIDENCE, retry.PACKET = HERE, EVIDENCE, PACKET
    retry.bind = lambda: validated
    retry.sibling = sibling
    retry.stage_request = stage_request
    retry.check_preflight = check_preflight
    return retry.entry()


if __name__ == "__main__":
    sys.exit(main())
