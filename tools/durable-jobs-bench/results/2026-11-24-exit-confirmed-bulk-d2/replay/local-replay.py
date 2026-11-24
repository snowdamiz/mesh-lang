#!/usr/bin/env python3
"""Reduce the archived local receipts without running a database or using a host lock."""
import hashlib
import json
import tarfile

from local import ORDERS, paired
from qualify import HERE, check_arm, check_faults, check_runner_log, check_teardown, digest


def normalize(value):
    return json.loads(json.dumps(value))


def verify_local(packet=HERE.parent):
    summary = json.loads((packet / "local-summary.json").read_text())
    with tarfile.open(packet / "local-raw.tar.gz") as archive:
        assert len(archive.getnames()) == len(set(archive.getnames()))

        def contents(prefix, name):
            entry = archive.getmember("./" + prefix + name)
            assert entry.isfile()
            with archive.extractfile(entry) as stream:
                return stream.read()

        def read(prefix, name):
            return json.loads(contents(prefix, name))

        for mode, name in (("off", "baseline-build.json"), ("d2", "d2-build.json")):
            build = contents("", name)
            assert hashlib.sha256(build).hexdigest() == summary["runners"][mode]["manifest"]
            assert json.loads(build)["files"]["mesh"] == summary["runners"][mode]["elf"]
        assert read("", "d2-build.json")["combined_source_sha256"] == digest(packet / "combined-source.tar.gz")

        def score(prefix, mode):
            selected = "off" if mode == "off" else "d2"
            check_runner_log(contents(prefix + "/", "runner-0.log").decode())
            return check_arm(lambda name: read(prefix + "/", name), mode,
                             summary["runners"][selected], native=False)

        assert summary["candidate_disabled"] == normalize(score("candidate-default", "d2-disabled"))
        groups = []
        for index, order in enumerate(ORDERS, 1):
            scores = {mode: score(f"group-{index}-{mode}", mode) for mode in order}
            groups.append(dict(group=index, order=order, scores=scores))
        assert normalize(groups) == summary["groups"]
        assert paired(groups) == summary["paired_percent"]
        for mode in ("off", "d2", "d2-forced"):
            receipt = read("teardown-" + mode + "/", "teardown.json")
            witness = read("", ("baseline" if mode == "off" else "d2") + "-witness-build.json")
            assert receipt["runner_sha256"] == witness["files"]["mesh"]
            assert receipt["broker_sha256"] == summary["broker"]["elf"]
            check_teardown(receipt, mode)
        for checkout, key in (("d2", "faults"), ("off", "off_faults")):
            assert check_faults(lambda name: read(checkout + "-faults/", name),
                                summary["runners"][checkout]["elf"], summary["broker"]["elf"],
                                checkout) == summary[key]

        def rejects(mutate_name, mutate):
            def load(name):
                value = read("group-1-d2/", name)
                if name == mutate_name:
                    mutate(value)
                return value

            try:
                check_arm(load, "d2", summary["runners"]["d2"], native=False)
            except AssertionError:
                return
            raise AssertionError(f"accepted falsified {mutate_name}")

        rejects("split-executables.json", lambda data: data.update(writer_mode="zero"))
        rejects("ledger.json", lambda data: data["jobs"][0].update(
            effect_xid=data["jobs"][0]["attempt_xid"]))
        rejects("producer-writer.json", lambda data: data.update(bulk=0, scalar=128))
        rejects("trusted-application-statements.json", lambda data: data.append(
            dict(query="SELECT $1", calls=256)))
    return dict(groups=len(groups), valid_arms=2 * len(groups) + 1,
                fault_receipts=len(summary["faults"]) + len(summary["off_faults"]),
                teardown_witnesses=3, refusal_controls=4)


if __name__ == "__main__":
    print(json.dumps(verify_local(), indent=2))
