#!/usr/bin/env python3
"""Qualify the composed plan against retained native archives without launching work."""
import json
import os
from pathlib import Path
import tarfile
from unittest.mock import patch

import controller as trial

matrix = trial.matrix
require = trial.require


def check_storage(config):
    packet = trial.STORAGE
    native = packet / "evidence/native"
    prior = matrix.read_json(packet / "frozen/native-config.json")
    summary = matrix.read_json(native / "summary.json")
    require(summary["completed_groups"] == [1, 2, 3, 4, 5], "EXTERNAL parent series incomplete")
    result = {}
    for number, record in summary["records"].items():
        block = native / record["attempt"] / f"{int(number):02d}-slow"
        for label, previous in {"one-a": "bulk-external", **trial.PEERS}.items():
            lane = prior["lanes"][previous]
            engine = lane["engine"]
            for phase in trial.PHASES:
                arm = block / f"{phase}-{previous}"
                folder = arm / f"{phase}-{engine}"
                retained = trial.prior_storage.score(arm, lane, prior, phase)
                require(retained == record["scores"][f"{phase}-{previous}"], "retained EXTERNAL score changed")
                with tarfile.open(folder / "raw.tar.gz") as raw:
                    clocks = trial.clocks(matrix.file_json(raw, "adapter/ledger.json"))
                if engine != "mesh":
                    combined = trial.score(folder, config, label, phase)
                    require(combined["late_refill_ms"] == clocks["late_refill_ms"], "peer score changed clocks")
                result[f"{number}-{phase}-{label}"] = dict(raw_sha256=retained["raw_sha256"], **clocks)
    return result


def check_topology():
    native = trial.TOPOLOGY / "evidence/native-import-retry"
    manifest = matrix.read_json(native / "series.json")
    config = manifest["config"]
    require(len(config["plan"]) == 6
            and config["cpus"] == [0, 1], "default topology manifest changed")
    checked = {}
    for block in config["plan"]:
        number = block["group"]
        paths = list((native / "blocks").glob(f"{number:02d}-slow.*"))
        complete = [path for path in paths if (path / "reconciled.json").is_file()]
        if not complete:
            require(number not in trial.PILOT, "missing original default topology pilot")
            continue
        path = sorted(complete)[0]
        for label in trial.MESH:
            inventory = matrix.read_json(native / "staging" / label / "inventory.json")
            for phase in trial.PHASES:
                arm = path / f"{phase}-{label}" / f"{phase}-mesh"
                metric = trial.prior_topology.score(arm, config, label, phase, inventory)
                require(metric["committed_hold_peak"] == 64, "retained topology lacked 64 holds")
                checked[f"{number}-{phase}-{label}"] = dict(
                    raw_sha256=metric["raw_sha256"], layout=metric["layout"],
                    committed_hold_peak=metric["committed_hold_peak"], app_xids=metric["application_xids"],
                    bulk_rows=metric["bulk_rows"], warm_roles=metric["warm_roles"])
    return checked


def check_composition():
    staged = trial.load("composition_topology", trial.PACKET / "stage/topology.py")
    for label, (runners, slots, apps) in staged.LAYOUTS.items():
        arm = dict(PROFILES={"slow": (1, 64, 8, 68, 2, staged.INPUT)}, h=None, Cluster=object, Subject=object)
        with patch.dict(os.environ, {"MESH_EXTERNAL_TOPOLOGY": label}):
            staged.install(arm, Path("."))
        require(arm["PROFILES"]["slow"] == (runners, slots*runners, apps, 68, 2, staged.INPUT)
                and arm["Cluster"] is not object and arm["Subject"] is not object, "topology hook drift")
    table = """  sl local_address rem_address st tx_queue tr tm->when retrnsmt uid timeout inode
  0: 0100007F:C310 0100007F:1538 01 00000000:00000000 00:00000000 00000000 1000 0 1456
  1: 0100007F:1538 0100007F:C310 01 00000000:00000000 00:00000000 00000000 1000 0 1457
  2: 0100007F:C311 0100007F:1538 06 00000000:00000000 00:00000000 00000000 1000 0 1458
"""
    require(staged.established_clients(table, 5432) == {"1456": 49936},
            "Linux socket parser matched server side or a closed client")

    def block(a, b, new):
        return {f"measured-{label}": dict(late_refill_ms=late, submit_ms=submit)
                for label, (late, submit) in zip(trial.MESH, (a, b, new), strict=True)}

    strong = block((1250, 1770), (1300, 1820), (1050, 1580))
    near = block((1250, 1770), (1300, 1820), (1220, 1750))
    submit_loss = block((1250, 1770), (1300, 1820), (1050, 1800))
    wide = block((1250, 1770), (1550, 2120), (1050, 1580))
    cases = {"one_only": ({1: strong}, False), "both_pass": ({1: strong, 2: strong}, True),
             "noise": ({1: near, 2: near}, False), "submit_loss": ({1: strong, 2: submit_loss}, False),
             "duplicate_spread": ({1: wide, 2: wide}, False)}
    for name, (groups, eligible) in cases.items():
        require(trial.continuation(groups)["eligible"] == eligible, f"incorrect continuation result for {name}")
    return list(cases)


def main():
    config = matrix.read_json(trial.FROZEN / "series.json")
    storage = check_storage(config)
    topology = check_topology()
    gate_checks = check_composition()
    print(json.dumps(dict(
        source_storage_arms=storage, source_topology_arms=topology, gate_checks=gate_checks,
        scope="Archive replay and component checks. No composed workload was started."
    ), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
