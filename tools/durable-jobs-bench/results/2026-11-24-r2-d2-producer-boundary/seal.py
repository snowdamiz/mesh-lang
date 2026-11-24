"""Mint the controller request from pinned source and recovered evidence."""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("stage_budget_controller", HERE / "controller.py")
controller = importlib.util.module_from_spec(spec)
spec.loader.exec_module(controller)
root = controller.ROOT
r2 = controller.R2
series = controller.SERIES
receipt = r2 / "recovery/evidence/qualification.json"
data = controller.run.read_json(receipt)
qualified = receipt.parent / data["attempt"] / "qualified-timing.json"
paths = [*HERE.glob("*.py"), *HERE.glob("*.sh"), HERE / "probe.rs",
         *sorted((HERE / "evidence/offline").glob("*.json")),
         qualified, receipt, r2 / "recovery/evidence/controller-release.json",
         series / "series.json", series / "SHA256SUMS",
         *(controller.R2_REPLAY / name for name in ("controller.py", "qualify.py", "split_arm.py", "combined-run", "identity.py")),
         controller.SPEC / "request.json", controller.SPEC / "short-capture.json", controller.SOURCE,
         Path(controller.matrix.__file__), *(controller.SUITE / name for name in ("run.py", "verify.py", "worker.py"))]
native = controller.native_pins(controller.required_configuration())
base_cmd = f"python3 -B {(HERE / 'controller.py').relative_to(root)}"
request = dict(
    id=controller.ID, controller="linode-native-stepchange-controller-v1", state="ready",
    operator="native-short-real-bulk-producer-stage-budget", classification="experimental attribution only",
    source_sha=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
    scope="Existing canonical 128-job R2 D2 producer. Baseline A/B, client instrumentation, and nested PG role statistics.",
    input_sha256=controller.INPUT,
    depends_on="Existing controller bookings, including enqueue-spec-preparation, release before this request.",
    cpus=[0, 1], expected_setup_minutes=18, requested_window_minutes=18,
    expected_timing_minutes=15, timing_window_minutes=15,
    setup_command=base_cmd + " setup", command=base_cmd + " setup",
    verify_setup_command=base_cmd + " verify-setup <attempt>",
    timing_command=base_cmd + " timing <attempt> 1 2",
    remaining_timing_command=base_cmd + " timing <attempt> 3 4 5",
    summarize_command=base_cmd + " replay <attempt>",
    offline_check_command=base_cmd + " check",
    evidence_destination=str((HERE / "evidence").relative_to(root)),
    setup_payload_sha256=sha256(controller.payload(native)).hexdigest(),
    groups=list(controller.plan(["short"], controller.LANES)),
    setup_grant_id=controller.ID, timing_grant_id=controller.ID,
    pins={str(path.relative_to(root)): controller.digest(path) for path in paths},
    native_pins=native,
)
print(json.dumps(request, indent=2, sort_keys=True))
