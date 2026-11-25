"""Read-only replay and sole-controller intake for the statement CPU diagnostic."""
from datetime import datetime, timedelta, timezone
import fcntl
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tarfile

from cpu_adapter import mesh_source
from measure import attribute, host_work, interpret, require

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
PREVIOUS = HERE.parent / "2026-11-24-dense-individual-hot-pool/logger-off-retry"
ARCHIVE = PREVIOUS / "evidence/native"
HOST_BUDGET = HERE.parent / "2026-11-25-dense-host-cpu-budget/bounds.json"
ID = HERE.name
PACKET = HERE / "request.json"
OUT = HERE / "evidence/native"
REMOTE = Path("/var/lib/durable-jobs-bench/native/dense-statement-cpu-20261125")
STAGE = ("cpu_adapter.py", "measure.py", "mesh-launch.sh", "smoke.py", "stmt_cpu.c")

sys.path.insert(0, str(PREVIOUS))
import retry

base, run = retry.base, retry.run
matrix, QUEUE = base.matrix, base.matrix.QUEUE
STOCK = {"mesh": "hot", "river": "river-2x32"}
ORDER = [f"{engine}-{case}" for case in ("off-a", "on", "off-b") for engine in STOCK]
BLOCKS = [dict(group=i, profile="noop", phases=["warmup", "measured"], order=order)
          for i, order in enumerate((ORDER, ORDER[::-1]), 1)]


def packet():
    request = run.read_json(PACKET)
    require(request["id"] == ID and request["schedule"] == BLOCKS and request["ready"], "request binding drift")
    for name, digest in request["pins"].items():
        require(run.digest(ROOT / name) == digest, f"sealed input changed: {name}")
    return request


def historical():
    retry.configure()
    retry.packet("timing")
    config = run.read_json(ARCHIVE / "series.json")["config"]
    binding = retry.relink_binding()
    require(config["product"] == binding, "released individual hot binding changed")
    result = run.read_json(ARCHIVE / "summary-1795570643531732000.json")
    require(result["qualified_rotations"] == 6 and not result["missing"], "reference lacks six rotations")
    return config, result


def stage(config, lane, remote, files):
    name, mode = lane.split("-", 1)
    stock = STOCK[name]
    engine = config["lanes"][stock]["engine"]
    original = run.read_json(ARCHIVE / "staging" / stock / "inventory.json")
    spec = original["engines"][engine]
    argv = spec["argv"]
    verified = {path: pin for path, pin in spec["verified_files"].items()
                if not (path.startswith(str(run.REMOTE) + "/") and path.endswith("/worker.py"))}
    verified.update({str(REMOTE / filename): files[filename]
                     for filename in ("cpu_adapter.py", "measure.py", "mesh-launch.sh", "stmt_cpu.so")})
    verified[str(remote / "worker.py")] = run.digest(base.dense.SUITE / "worker.py")
    enabled = "on" if mode == "on" else "off"
    if engine == "mesh":
        require(argv[-2:] == [str(retry.STAGE / "native-run"), "mesh"], "historical Mesh launcher changed")
        argv = [*argv[:-2], str(REMOTE / "mesh-launch.sh"), enabled, "mesh"]
    else:
        require(argv[:2] == ["/usr/bin/python3", "-B"] and argv[3] == "go" and spec["run_as"] == "nobody",
                "historical River launcher changed")
        argv = [*argv[:2], str(REMOTE / "cpu_adapter.py"), "go", enabled, argv[2], *argv[3:]]
    spec.update(argv=argv, verified_files=verified,
                evidence_globs=[*spec["evidence_globs"], "dense-statement-keys.json", "dense-statement-cpu.jsonl"])
    run.check_inventory(original)
    return [base.dense.SUITE / "worker.py"], original


def events():
    return re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                      (QUEUE / "STATUS.md").read_text(), re.M)


def registered():
    require(PACKET.read_bytes() == (QUEUE / f"{ID}.json").read_bytes(), "controller intake differs from seal")


def grant(mode):
    registered()
    kind, ident, start, end = events()[-1]
    require((kind, ident) == (mode, f"{ID}-{mode.lower()}")
            and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "active matching controller grant required")
    return dict(mode=mode, id=ident, start=start, end=end, request_sha256=run.digest(PACKET))


def remaining(mode):
    return (datetime.fromisoformat(grant(mode)["end"]) - datetime.now(timezone.utc)).total_seconds()


def payload():
    archive = BytesIO()
    with tarfile.open(fileobj=archive, mode="w") as bundle:
        checksum = []
        for name in STAGE:
            data = (HERE / name).read_bytes()
            checksum.append(f"{sha256(data).hexdigest()}  {name}\n")
            member = tarfile.TarInfo(name)
            member.size = len(data)
            bundle.addfile(member, BytesIO(data))
        data = "".join(checksum).encode()
        member = tarfile.TarInfo("input.sha256")
        member.size = len(data)
        bundle.addfile(member, BytesIO(data))
    return archive.getvalue()


def setup_files():
    receipt = run.read_json(OUT / "setup/receipt.json")
    files = receipt["files"]
    require(receipt["version"] == "17.11" and receipt["platform"] == "linux-x86_64"
            and receipt["remote"] == str(REMOTE)
            and all(files[name] == run.digest(HERE / name) for name in STAGE)
            and re.fullmatch("[0-9a-f]{64}", files["stmt_cpu.so"])
            and [s["enabled"] for s in receipt["smoke"]] == [False, True]
            and all(s["sessions"] == 1 and "backend-exit" in s["tested"] for s in receipt["smoke"])
            and {"nested-spi", "abort", "pgss-reset", "client-idle"} <= set(receipt["smoke"][1]["tested"]),
            "Linux SETUP proof did not qualify")
    return files


def release(mode):
    document = run.read_json(OUT / mode / "controller-release.json")
    require(document["id"] == f"{ID}-{mode}" and document["request_sha256"] == run.digest(PACKET)
            and document["exit_status"] == 0 and document["released_at"], f"missing clean {mode} release")


def setup():
    folder = OUT / "setup"
    folder.mkdir(parents=True, exist_ok=True)
    with (base.dense.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        require(remaining("SETUP") >= 120, "SETUP window too short")
        run.write_json(folder / "grant.json", grant("SETUP"))
        with (folder / "setup.log").open("xb") as log, (folder / "setup.stdout").open("xb") as stdout:
            result = run.ssh(["timeout", "--signal=TERM", "--kill-after=15s", "100s",
                              "bash", "-c", (HERE / "setup.sh").read_text()],
                             timeout=119, input=payload(), stdout=stdout, stderr=log)
    result.check_returncode()
    run.write_json(folder / "receipt.json", run.read_json(folder / "setup.stdout"))
    setup_files()


def score_arm(folder, phase, lane, config, remote, files):
    _, inventory = stage(config, lane, remote, files)
    name, mode = lane.split("-", 1)
    original = base.stages
    try:
        base.stages = lambda *_: ([], inventory)
        score = base.score_arm(folder, phase, STOCK[name], config, remote)
    finally:
        base.stages = original
    with tarfile.open(folder / "raw.tar.gz") as raw:
        catalog = run.file_json(raw, "adapter/dense-statement-keys.json")
        require(catalog["module_sha256"] == files["stmt_cpu.so"]
                and catalog["configured_us"] < score["first_submit_us"] < score["last_settlement_us"]
                < catalog["dictionary_us"] <= catalog["published_us"], "CPU adapter clock/artifact drift")
        roster = run.file_json(raw, "adapter/postgres-tls-sessions.json") if name == "mesh" else []
        report = raw.extractfile("adapter/dense-statement-cpu.jsonl").read().decode()
        postgres = attribute(report, catalog, mode == "on", name, roster)
        points = [json.loads(line) for line in raw.extractfile("host-samples.jsonl")]
    host = host_work(points, catalog["configured_us"], catalog["published_us"])
    require(postgres["postgres_cpu_sample_ns"] <= host["host_upper_ns"], "PostgreSQL exceeds host CPU bracket")
    return dict(raw_sha256=score["raw_sha256"], phase=phase, cluster_id=catalog["cluster_id"],
                submit_ns=(score["last_settlement_us"] - score["first_submit_us"]) * 1000,
                drain_ns=(score["last_settlement_us"] - score["first_commit_us"]) * 1000,
                interval=[points[0]["monotonic_ns"], points[-1]["monotonic_ns"]], **host,
                host_minus_sampled_postgres_ns=host["host_busy_ns"] - postgres["postgres_cpu_sample_ns"],
                postgres=postgres)


def historical_gap():
    prior = run.read_json(HOST_BUDGET)
    # At 1.2x River throughput the two pinned CPUs have 5/3 times River's elapsed microseconds.
    return [(3000 * arms["hot"]["submit_busy_floor_cpu_us"] - 5000 * arms["river-2x32"]["submit_us"]) // 3
            for arms in prior.values()]


def replay_result(cfg, files):
    directory = OUT / "timing"
    manifest = run.read_json(directory / "series.json")
    require(manifest["schedule"] == BLOCKS and manifest["pins"] == timing_pins(), "timing schedule or seal changed")
    arms = {}
    all_scores = []
    for block in BLOCKS:
        attempts = list((directory / "blocks").glob(f"{matrix.run.block_prefix(block)}.*"))
        require(len(attempts) == 1 and run.read_json(attempts[0] / "order.json") == block, "rotation attempt/order differs")
        group = {}
        for phase, lane, _, folder in matrix.paths(attempts[0], manifest["config"], block):
            score = score_arm(folder, phase, lane, cfg, run.REMOTE / manifest["remotes"][lane], files)
            all_scores.append(score)
            if phase == "measured":
                group[lane] = score
        arms[str(block["group"])] = group
    intervals = [t for score in all_scores for t in score["interval"]]
    require(intervals == sorted(set(intervals)) and len({s["cluster_id"] for s in all_scores}) == len(all_scores),
            "arms overlap or PostgreSQL cluster reused")
    return dict(measured=arms, warmup_raw_sha256=[s["raw_sha256"] for s in all_scores
                                                if s["phase"] == "warmup"],
                **interpret(arms, historical_gap()))


def timing_pins():
    return dict(request_sha256=run.digest(PACKET), setup_sha256=run.digest(OUT / "setup/receipt.json"),
                setup_release_sha256=run.digest(OUT / "setup/controller-release.json"))


def timing(cfg, files):
    release("setup")
    grant("TIMING")
    directory = OUT / "timing"
    require(not (directory / "series.json").exists(), "prior timing attempt needs sole-controller reconciliation")
    config = dict(cfg, lanes={lane: cfg["lanes"][STOCK[lane.split("-", 1)[0]]] for lane in ORDER})
    matrix.stages = lambda _, lane, remote: stage(cfg, lane, remote, files)
    matrix.grant = lambda _: grant("TIMING")
    matrix.SERIES[ID] = ID

    def replay_block(path, block, _, remotes):
        require(run.read_json(path / "order.json") == block, "native rotation order differs")
        return {f"{phase}-{lane}": score_arm(folder, phase, lane, cfg, run.REMOTE / remotes[lane], files)
                for phase, lane, _, folder in matrix.paths(path, config, block)}

    matrix.replay_block = replay_block
    matrix.summarize = lambda *_: dict(missing=[], **replay_result(cfg, files))
    ssh = run.ssh
    dispatch = run.dispatch_arm

    def within_grant(args, *, timeout, **options):
        left = remaining("TIMING") - 4
        require(left > 0, "TIMING assignment ended")
        return ssh(args, timeout=min(timeout, left), **options)

    def checked_dispatch(path, remote, block, phase, engine):
        require(remaining("TIMING") >= 110, "insufficient booked time for another arm")
        dispatch(path, remote, block, phase, engine)
        require(run.read_json(path / f"{phase}-{engine}/dispatch.json").get("exit_code") == 0,
                "preserve failed arm for sole-controller reconciliation")

    run.ssh, run.dispatch_arm = within_grant, checked_dispatch
    try:
        return matrix.dispatch(directory, config, ID, BLOCKS, timing_pins(), [1, 2])
    finally:
        run.ssh, run.dispatch_arm = ssh, dispatch


def check(cfg, history):
    for filename in ("mesh-launch.sh", "setup.sh"):
        subprocess.run(["bash", "-n", HERE / filename], check=True)
    original = (PREVIOUS / "native-run").read_text()
    expected = original.replace('here="$(cd "$(dirname "$0")" && pwd -P)"',
                                f'here="$base/{retry.STAGE.name}"\nmode="$1"\nshift\n[[ "$mode" == on || "$mode" == off ]]')
    expected = expected.replace('python3 -B "$here/adapter.py" "$@"',
                                'python3 -B "$base/dense-statement-cpu-20261125/cpu_adapter.py" mesh "$mode" "$here/adapter.py" "$@"')
    require((HERE / "mesh-launch.sh").read_text() == expected, "stock Mesh wrapper changed")
    source = PREVIOUS.parent / "adapter.py"
    compile(mesh_source(source.read_text()), str(source), "exec")
    files = {name: run.digest(HERE / name) for name in STAGE} | {"stmt_cpu.so": "0" * 64}
    for lane in ORDER:
        stage(cfg, lane, run.REMOTE / lane, files)
    for kind, stock in STOCK.items():
        key = f"measured-{stock}"
        block = history["blocks"]["1"]
        folder, = (ARCHIVE / "blocks" / block["attempt"] / key).glob("*/raw.tar.gz")
        remotes = run.read_json(ARCHIVE / "series.json")["remotes"]
        scored = base.score_arm(folder.parent, "measured", stock, cfg, run.REMOTE / remotes[stock])
        require(scored["completed"] == 1024 and block["scores"][key]["raw_sha256"] == scored["raw_sha256"],
                f"released {kind} raw replay differs")
    print(json.dumps(dict(request_sha256=run.digest(PACKET), payload_sha256=sha256(payload()).hexdigest(),
                          historical_gap_ns=historical_gap())))


def intake(mode, seal, request):
    registered()
    require(seal == run.digest(PACKET) and events()[-1][0] == "RELEASED", "intake seal or prior release differs")
    if mode == "timing":
        release("setup")
        setup_files()
    folder = OUT / mode
    folder.mkdir(parents=True, exist_ok=False)
    registry = runpy.run_path(str(QUEUE.parent / "hot-checkout-slots.py"))
    registry["probe"]()
    start = datetime.now(timezone.utc)
    end = start + timedelta(minutes=request["minutes"][mode])
    ident = f"{ID}-{mode}"
    registry["note"](f"{mode.upper()} {ident} {start:%Y-%m-%dT%H:%M:%SZ} {end:%Y-%m-%dT%H:%M:%SZ}\n\n"
                     f"Owner: sole native controller. Sealed request {seal}.")
    status = 1
    try:
        with (folder / "controller.log").open("xb") as log:
            status = subprocess.run([sys.executable, "-B", __file__, mode], cwd=ROOT,
                                    stdout=log, stderr=log).returncode
    finally:
        registry["probe"]()
        result = dict(id=ident, exit_status=status, request_sha256=seal,
                      started_at=start.isoformat(), released_at=run.utc_now())
        run.write_json(folder / "controller-release.json", result)
        registry["note"](f"RELEASED {ident} {result['released_at']}, exit {status}. Evidence: {folder}")
    return status


if __name__ == "__main__":
    action, *args = sys.argv[1:]
    request = packet()
    cfg, history = historical()
    if action == "check":
        check(cfg, history)
    elif action == "intake":
        mode, seal = args
        require(mode in ("setup", "timing"), "intake requires setup or timing")
        sys.exit(intake(mode, seal, request))
    elif action == "setup":
        setup()
    elif action == "timing":
        sys.exit(timing(cfg, setup_files()))
    elif action == "replay":
        release("setup")
        release("timing")
        print(json.dumps(replay_result(cfg, setup_files()), indent=2, sort_keys=True, allow_nan=False))
    else:
        raise SystemExit("expected check, intake, setup, timing or replay")
