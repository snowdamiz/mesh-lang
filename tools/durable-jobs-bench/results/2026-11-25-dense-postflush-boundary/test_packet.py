"""Offline checks of the packet and its failure boundaries."""
import io
import json
from pathlib import Path
import shutil
import tarfile
import tempfile
import unittest

from boundary import join, lift_xid, parse_export, validate_clock, XID_RANGE
from packet import BASE, CANARY, ORDERS, PG, POOL, ROOT, RUNNER, checked, inventory, run
from score import perturbation, score_arm

SETUP = dict(pg={"version": "PostgreSQL 17.11", "bindir": PG,
                 "includedir-server": PG.removesuffix("/bin") + "/include/server",
                 "pkglibdir": PG.removesuffix("/bin") + "/lib/postgresql"}, module=BASE + "/postflush-pg17-123456789abc/postflush.so",
             sha256="a" * 64, canary=CANARY)


def clocks():
    return dict(start=dict(clock=[1000, 1_000_000, 1009], before_ns=1_000_001, after_ns=1_000_900,
                           sql_us=1000, xid=str(XID_RANGE - 5), system_identifier="73"),
                end=dict(clock=[100_000, 1_099_000, 100_009], before_ns=1_099_001, after_ns=1_099_900,
                         sql_us=1099, xid=str(XID_RANGE + 12), system_identifier="73"))


def backlog():
    # Deliberately place producer, application, and settlement around XID wrap.
    metadata = dict(clocks(), enabled=True, parallel_workers=8, max_prepared_transactions=0,
                    synchronous_standby_names="", wal_sync_method="fdatasync", exports_collected_ns=1_110_000)
    db = dict(version="17.11", fsync=True, full_page_writes=True, synchronous_commit=True,
              track_commit_timestamp=True, tls=True)
    job = dict(id="j", worker_pid=33, job_key="k", attempt=1, attempts=1, effects=1, state="completed",
               entered_us=1010, settled_us=1060, settled_xid="4",
               attempt_xid=str(XID_RANGE - 1), attempt_committed_us=1020, effect_xid="3", effect_committed_us=1040)
    witness = dict(job_id="j", job_key="k", attempt=1, entered_us=1010, lookup_begin_us=1012,
                   lookup_end_us=1013, first_return_us=1030, second_return_us=1050)
    log = "APP_POOL_JOB " + json.dumps(witness)
    ledger = dict(database=db, transactions=[dict(xid=str(XID_RANGE - 3), submitted_us=1004)], jobs=[job])
    roster = [dict(role="compiled_caller", backend=pid, tls=True) for pid in range(100, 132)]
    exports = {pid: f"postflush-v1 {pid} 1 0 1001900 1099900\nend 0 0 0 0\n" for pid in range(100, 132)}
    exports[100] = f"postflush-v1 100 1 0 1001900 1099900\n{XID_RANGE - 1} 1024500\nend 1 0 0 0\n"
    exports[101] = f"postflush-v1 101 1 0 1001900 1099900\n{XID_RANGE + 3} 1046800\nend 1 0 0 0\n"
    return ledger, [33, 34], [log, ""], roster, roster, metadata, exports, True, 1


class PacketTest(unittest.TestCase):
    def test_packet_pins_real_prior_runner(self):
        seal = checked()
        self.assertEqual(sum(len(b["order"]) * len(b["phases"]) for b in seal["groups"]), 18)
        inventories = {}
        for lane in ORDERS[0]:
            stage = f"/var/tmp/mesh-durable-jobs-matrix/postflush-123456789abc-{lane}"
            spec = inventory(SETUP, stage, lane)["engines"]["mesh"]
            self.assertEqual(spec["argv"][3:], [RUNNER, SETUP["module"]])
            self.assertEqual(spec["verified_files"][SETUP["module"]], SETUP["sha256"])
            inventories[lane] = spec
        self.assertEqual(inventories["off-a"]["argv"][2:], inventories["off-b"]["argv"][2:])
        self.assertEqual(inventories["on"]["argv"][2], "on")
        for key in ("go", "oban", "rust"):
            self.assertEqual(inventory(SETUP, "/var/tmp/mesh-durable-jobs-matrix/postflush-123456789abc-on",
                                       "on")["engines"][key], run.read_json(
                POOL / "evidence/timing/grant-1/direct-observed/inventory.json")["engines"][key])

    def test_reject_unqualified_module_or_stage(self):
        stage = "/var/tmp/mesh-durable-jobs-matrix/postflush-123456789abc-on"
        for change in ({"sha256": "bad"}, {"canary": {}}, {"module": PG + "/postflush.so"}):
            with self.assertRaisesRegex(ValueError, "unqualified"):
                inventory(dict(SETUP, **change), stage, "on")
        with self.assertRaisesRegex(ValueError, "invalid private stage"):
            inventory(SETUP, stage, "off-a")

    def test_bind_across_wrap(self):
        rows = join(*backlog())
        self.assertEqual([(row["xid"], row["backend"], row["record_to_callback_us"], row["callback_to_return_us"])
                          for row in rows], [(XID_RANGE - 1, 100, 4, 6), (XID_RANGE + 3, 101, 6, 4)])

    def test_bad_clocks_and_epoch_range(self):
        for field, value in (("system_identifier", "81"), ("xid", str(3 * XID_RANGE)),
                             ("clock", [100_000, 2_000_000, 100_009]), ("sql_us", 1010)):
            data = clocks()
            data["end"][field] = value
            with self.assertRaises(ValueError):
                validate_clock(data["start"], data["end"])
        with self.assertRaisesRegex(ValueError, "outside"):
            lift_xid("34", (XID_RANGE - 5, XID_RANGE + 12))

    def test_export_errors(self):
        sample = backlog()[6][101]
        for last in ("end 1 1 0 0\n", "end 1 0 1 0\n", "end 1 0 0 1\n", "end 0 0 0 0\n", ""):
            with self.subTest(last=last), self.assertRaises(ValueError):
                parse_export(sample[:sample.index("end")] + last, 101, True)
        with self.assertRaisesRegex(ValueError, "exit status"):
            parse_export(sample.replace("101 1 0", "101 1 1"), 101, True)
        with self.assertRaisesRegex(ValueError, "inactive"):
            parse_export(sample.replace("101 1 0", "101 0 0"), 101, False)

    def test_missing_duplicate_or_late_commit(self):
        args = backlog()
        del args[6][100]
        with self.assertRaisesRegex(ValueError, "missing, extra or replaced"):
            join(*args)
        args = backlog()
        args[6][102] = args[6][101].replace("v1 101", "v1 102")
        with self.assertRaisesRegex(ValueError, "global duplicate"):
            join(*args)
        args = backlog()
        args[6][101] = args[6][101].replace("1046800", "1052000")
        with self.assertRaisesRegex(ValueError, "outside commit-record"):
            join(*args)

    def test_reject_wrong_producer_settlement_or_handler(self):
        args = backlog()
        args[0]["jobs"][0]["settled_xid"] = "3"
        with self.assertRaisesRegex(ValueError, "producer/settlement"):
            join(*args)
        args = backlog()
        args[2][1] = args[2][0]
        with self.assertRaisesRegex(ValueError, "repeated handler"):
            join(*args)

    def test_perturbation_gate(self):
        def score(n):
            delta = {"record_to_return_us": {"p95": n}}
            return dict(drain_us=n, split=dict(attempt=delta, effect=delta))

        self.assertTrue(all(v["pass_gate"] for v in perturbation(
            {"on": score(100), "off-a": score(98), "off-b": score(102)}).values()))
        self.assertFalse(any(v["pass_gate"] for v in perturbation(
            {"on": score(112), "off-a": score(98), "off-b": score(102)}).values()))
        self.assertFalse(any(v["pass_gate"] for v in perturbation(
            {"on": score(100), "off-a": score(90), "off-b": score(110)}).values()))

    def test_retained_ledger_and_handler_schema(self):
        root = POOL / "timing-clean-identity-retry/evidence/timing/01-noop"
        path = root / "measured-direct-observed/measured-mesh/raw.tar.gz"
        if not path.is_file():
            self.skipTest("prior controller archive is held separately")
        raw_sha = run.read_json(root / "reconciled.json")["measured"]["direct-observed"]["raw_sha256"]
        self.assertEqual(run.digest(path), raw_sha)
        with tarfile.open(path) as archive:
            def read(name):
                return run.file_json(archive, "adapter/" + name + ".json")
            ledger, identity = read("ledger"), read("runner-identity")
            pids = [row["pid"] for row in identity["runners"]]
            logs = [archive.extractfile(f"adapter/runner-{i}.log").read().decode() for i in range(2)]
            before, after = read("sessions-before"), read("sessions-end")
        # Synthetic brackets and inactive receipts exercise the real ledger and log schema.
        producer = ledger["transactions"][0]
        first = producer["submitted_us"] - 1_000_000
        last = max(job["settled_us"] for job in ledger["jobs"]) + 1000
        def anchor(us, xid):
            return dict(clock=[us * 1000 - 2000, us * 1000 - 1000, us * 1000 - 1999],
                        before_ns=us * 1000, after_ns=us * 1000 + 500, sql_us=us,
                        system_identifier="73", xid=str(xid))
        meta = dict(start=anchor(first, 3), end=anchor(last, 1 << 31), enabled=False, parallel_workers=8,
                    max_prepared_transactions=0, synchronous_standby_names="", wal_sync_method="fdatasync",
                    exports_collected_ns=(last + 100) * 1000)
        pid_exports = {r["backend"]: f"postflush-v1 {r['backend']} 0 0 {first*1000} {last*1000}\nend 0 0 0 0\n"
                       for r in before if r["role"] == "compiled_caller"}
        joined = join(ledger, pids, logs, before, after, meta, pid_exports, False)
        self.assertEqual(len(joined), 2048)
        with self.assertRaisesRegex(ValueError, "missing, extra or replaced"):
            join(ledger, pids, logs, before, after, meta, {}, False)

        # Run the scorer's archive entry point with the synthetic hook records.
        with tempfile.TemporaryDirectory(prefix="postflush-replay-", dir=ROOT / "target") as tmp:
            out = Path(tmp)
            with tarfile.open(path) as prior, tarfile.open(out / "raw.tar.gz", "w:gz") as updated:
                for member in prior.getmembers():
                    data = prior.extractfile(member).read()
                    if member.name == "adapter.log":
                        data += b"\nPOSTFLUSH_STAGED_HELPERS_READABLE\n"
                    member.size = len(data)
                    updated.addfile(member, io.BytesIO(data))
                additional = {"adapter/postflush-arm.json": json.dumps(meta),
                              **{f"adapter/postflush-{pid}.log": body for pid, body in pid_exports.items()}}
                for name, content in additional.items():
                    data = content.encode()
                    member = tarfile.TarInfo(name)
                    member.size = len(data)
                    updated.addfile(member, io.BytesIO(data))
            for name in ("dispatch.json", "request.json"):
                shutil.copyfile(path.parent / name, out / name)
            expected = run.read_json(root.parent / "grant-1/direct-observed/inventory.json")
            full = score_arm(out, "measured", expected, False)
            self.assertEqual(full["split"]["both"]["n"], 2048)
            with self.assertRaisesRegex(ValueError, "hook mode"):
                score_arm(out, "measured", expected, True)


if __name__ == "__main__":
    unittest.main()
