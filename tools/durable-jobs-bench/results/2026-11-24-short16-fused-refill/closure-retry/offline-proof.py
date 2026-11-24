"""Unprivileged offline proof: original vs corrected short16 adapter, driver and launcher binding.

Usage (checkout root): python3 -B <this> <new-output-dir> <frozen ordinary-v2 harness>
No SSH, no locks, no benchmark: run.ssh and fcntl.flock are replaced by failing stubs.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from datetime import datetime, timedelta, timezone

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[4]
ID = "2026-11-24-short16-fused-refill"
PACKET = HERE.parent
REPLAY = ROOT / "target/short16-fused-refill/source/tools/durable-jobs-bench/results" / ID / "replay"
PUBLIC_MANIFEST = ROOT / ("tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/"
                          "evidence/native/setup/trusted-jobs-product-623c46b92cc5/mesh.json")
ATTEMPT = "short16-fused-refill-c4c5cc2243b6"

ADAPTER_PROBE = r'''
import json, sys, tempfile
from pathlib import Path
sys.path[:0] = [sys.argv[1], sys.argv[2]]
import backend
backend.install()
arm = backend.arm
parent = arm.Cluster.__mro__[1]
class Reached(Exception):
    pass
def sentinel(self, *args):
    raise Reached(type(self).__name__)
parent.reconcile_trial = sentinel
cluster = object.__new__(backend.Cluster)
cluster.evidence = Path(tempfile.mkdtemp(dir=sys.argv[3]))
cluster.sql = lambda engine, query: '{"attempts": [], "effects": []}'
try:
    cluster.reconcile_trial("mesh", "parity_0123456789abcdef", {}, {}, ())
    result = dict(outcome="returned")
except Reached as reached:
    result = dict(outcome="reached_superclass", superclass=f"{parent.__module__}.{parent.__qualname__}",
                  instance=str(reached), application_records=(cluster.evidence / "application-records.json").is_file())
except RuntimeError as error:
    result = dict(outcome="runtime_error", error=str(error))
print(json.dumps(result))
'''


def adapter(out, frozen, directory):
    env = dict(os.environ, PRODUCT_FROZEN_HARNESS=str(frozen), PRODUCT_COMPOSITION="product-fused")
    text = subprocess.check_output([sys.executable, "-B", "-c", ADAPTER_PROBE, str(directory), str(REPLAY), str(out)],
                                   env=env, cwd=out, text=True)
    return json.loads(text)


def fake_requests(out, grant, bound, public_manifest_sha):
    requests = out / f"requests-{grant}"
    requests.mkdir()
    now = datetime.now(timezone.utc)
    (requests / "STATUS.md").write_text(
        f"TIMING {grant} {now - timedelta(minutes=1):%Y-%m-%dT%H:%M:%SZ} {now + timedelta(minutes=30):%Y-%m-%dT%H:%M:%SZ}\n")
    shutil.copyfile(ROOT / f"target/linode-jobs-stepchange/requests/{ID}.json", requests / f"{ID}.json")
    for name in (f"{ID}.released.json", f"{ID}-closure-retry.released.json"):
        (requests / name).write_text(json.dumps(dict(id=name.removesuffix(".released.json"), exit_status=0)))
    (requests / f"{ID}.public-control.json").write_text(json.dumps(dict(
        dependency="2026-11-24-trusted-jobs-product-composition", manifest_sha256=public_manifest_sha,
        receipts={str(PUBLIC_MANIFEST): public_manifest_sha})))
    return requests


def driver(out, amended, requests, bound, groups):
    code = r'''
import fcntl, json, runpy, sys
from pathlib import Path
mode, driver, amendment_path, requests, bound, output, *groups = sys.argv[1:]
replay = Path(driver).parent
sys.path[:0] = [str(replay), str(replay.parents[2])]
import run
def forbidden(*args, **kwargs):
    raise AssertionError("offline proof attempted host or lock access")
run.ssh = forbidden
fcntl.flock = forbidden
if mode == "amended":
    amendment = runpy.run_path(amendment_path)
    source = amendment["amended_source"]()
    namespace = dict(__file__=driver, __name__="offline", _closure_retry_validate=amendment["eight_lane_validate"])
else:
    source = Path(driver).read_text()
    namespace = dict(__file__=driver, __name__="offline")
exec(compile(source, driver, "exec"), namespace)
matrix = namespace["matrix"]
def dispatch(directory, config, selected):
    matrix.validate(config)
    return 0
matrix.dispatch = dispatch
sys.argv = [driver, requests, bound, output, *groups]
try:
    result = dict(outcome="validated", exit=namespace["main"]())
except Exception as error:
    result = dict(outcome=type(error).__name__, error=str(error))
print(json.dumps(result))
'''
    output = out / f"timing-{amended}-{Path(requests).name}-{'-'.join(groups)}"
    text = subprocess.check_output([sys.executable, "-B", "-c", code, amended, str(REPLAY / "controller-timing.py"),
                                    str(HERE / "controller-timing.py"), str(requests), str(bound), str(output),
                                    *groups], cwd=ROOT, text=True)
    return json.loads(text)


def main():
    out, frozen = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve(strict=True)
    out.mkdir()
    sys.path[:0] = [str(REPLAY), str(REPLAY.parents[2])]
    import run
    report = dict(frozen_arm_sha256=run.digest(frozen / "tools/durable-jobs-bench/native/arm.py"))
    assert report["frozen_arm_sha256"] == "d9f984dad988e2380b22e971ae5abf122619f246bf592a9402720d49996685ad"

    # 1. Adapter: the archived original backend loses the class cell; the overlay reaches arm.Cluster's base.
    report["adapter"] = dict(original=adapter(out, frozen, REPLAY), corrected=adapter(out, frozen, HERE),
                             original_sha256=run.digest(REPLAY / "backend.py"),
                             corrected_sha256=run.digest(HERE / "backend.py"))
    assert report["adapter"]["original"]["outcome"] == "runtime_error"
    assert "__class__" in report["adapter"]["original"]["error"]
    assert report["adapter"]["corrected"]["outcome"] == "reached_superclass"

    # 2. Bind with the original sealed intake: real public manifest, synthetic candidate (pre-SETUP).
    build = json.loads(PUBLIC_MANIFEST.read_text())
    candidate = dict(build, source_sha256="c4c5cc2243b6db3a9f7d50cf5f25b115a8c54e356cb876503b0d709b46d2c422",
                     files=dict(build["files"], mesh="0" * 64))
    synthetic = out / "synthetic-candidate-mesh.json"
    synthetic.write_text(json.dumps(candidate, indent=2, sort_keys=True))
    bound = out / "short.inventory.json"
    bound.write_bytes(subprocess.check_output([sys.executable, "-B", str(REPLAY / "intake.py"), "bind", ATTEMPT,
                                               str(synthetic), "trusted-jobs-product-623c46b92cc5",
                                               str(PUBLIC_MANIFEST)], cwd=ROOT))
    public_sha = run.digest(PUBLIC_MANIFEST)

    # 3. Corrected launcher rebinding (imports the corrected launcher; its module load verifies the pinned
    #    controller launcher but performs no host action).
    os.chdir(ROOT)
    import runpy
    launcher = runpy.run_path(str(HERE / "launcher.py"))
    rebound_doc = launcher["rebind"](json.loads(bound.read_text()))
    rebound = out / "short.closure-retry.inventory.json"
    rebound.write_text(json.dumps(rebound_doc, indent=2, sort_keys=True) + "\n")
    mesh = {label: lane["inventory"]["engines"]["mesh"]["argv"][-2]
            for label, lane in rebound_doc["configuration"]["lanes"].items() if lane["engine"] == "mesh"}
    report["rebound_launchers"] = mesh
    assert all("/replay-closure-retry/native-run" in path for path in mesh.values()) and len(mesh) == 5

    # 4. Drivers: original driver on the original binding, then amended driver on the rebinding.
    original_requests = fake_requests(out, ID, bound, public_sha)
    amended_requests = fake_requests(out, f"{ID}-closure-retry", rebound, public_sha)
    report["driver"] = dict(
        original_pilot=driver(out, "original", original_requests, bound, ["1", "2"]),
        amended_pilot=driver(out, "amended", amended_requests, rebound, ["1", "2"]),
        amended_on_original_binding=driver(out, "amended", amended_requests, bound, ["1", "2"]),
        amended_groups_3_5_without_pilot=driver(out, "amended", amended_requests, rebound, ["3", "4", "5"]),
    )
    d = report["driver"]
    assert d["original_pilot"]["outcome"] == "ValueError" and "six lanes" in d["original_pilot"]["error"], d
    assert d["amended_pilot"] == dict(outcome="validated", exit=0), d
    assert d["amended_on_original_binding"] == dict(
        outcome="AssertionError", error="mesh lane is not bound to the closure-retry adapter"), d
    assert d["amended_groups_3_5_without_pilot"]["outcome"] == "AssertionError" \
        and "reconcile groups 1 and 2 first" in d["amended_groups_3_5_without_pilot"]["error"], d

    # Lane-set strictness of the eight-lane validator itself.
    from importlib import import_module
    sys.path.insert(0, str(REPLAY.parents[1] / "2026-11-23-bulk-enqueue-v2-short/replay"))
    matrix = import_module("interleave-v2")
    amend = runpy.run_path(str(HERE / "controller-timing.py"))
    amend["eight_lane_validate"](matrix)
    config = rebound_doc["configuration"]
    matrix.validate(config)
    seven = dict(config, lanes={k: v for k, v in config["lanes"].items() if k != "fused-b"})
    renamed = dict(config, lanes={("fused-c" if k == "fused-b" else k): v for k, v in config["lanes"].items()})
    for name, variant in (("seven", seven), ("renamed", renamed)):
        try:
            matrix.validate(variant)
            report.setdefault("lane_set", {})[name] = "accepted"
        except ValueError as error:
            report.setdefault("lane_set", {})[name] = str(error)
    assert all(v == "use exactly the frozen eight short16 lanes" for v in report["lane_set"].values()), report
    report["inputs"] = {str(p.relative_to(ROOT)): run.digest(p) for p in (
        REPLAY / "backend.py", REPLAY / "controller-timing.py", REPLAY / "intake.py", PUBLIC_MANIFEST,
        HERE / "backend.py", HERE / "controller-timing.py", HERE / "launcher.py", HERE / "setup.sh",
        ROOT / "target/linode-jobs-stepchange/short16-fused-refill-slots.py")}
    (out / "offline-proof.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps(dict(adapter={k: v["outcome"] for k, v in report["adapter"].items() if isinstance(v, dict)},
                          driver={k: v["outcome"] for k, v in d.items()}, lane_set=report["lane_set"]), indent=2))


if __name__ == "__main__":
    main()
