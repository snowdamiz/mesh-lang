# Ordinary Mesh native CPU clock correction

The old ordinary V1 diagnostic called a CPython `time` attribute that does not exist. The retained Linux CPython 3.14.4 run reproduces the failure while loading the canonical worker pinned by the old request. The controller held that request before native dispatch. [Failed Linux smoke](evidence/linux-red/serial.log) · [Previous seal](previous-clock-seal/request.json)

The recorder now binds `clock_getcpuclockid` from libc via ctypes, uses signed `clockid_t`, and passes it to `time.clock_gettime_ns`. All charged PIDs get a pidfd liveness check and a `/proc/PID/stat` identity check on both sides of the clock read. A PID that exits or changes start time, parent, or process name invalidates the frame. The native replay also verifies the canonical worker hash reported by the loaded module. The binding follows the earlier [protected-worker fix](../2026-11-23-native-worker-cost/replay/runner-clock.patch).

The corrected staged collector passed on the existing local ARM64 Linux sandbox (kernel 6.12.115-0-virt, CPython 3.14.4). It loaded the exact canonical worker, ran the staged entry point, sampled CPU progress in a child owned by UID 65534, and rejected the child when it exited during the read and after reap. The macOS regression exercises the default sampler and signed clock ID through a libc seam. Its remaining checks revalidate the five archived dense ledgers and reject changed process identities, clock inconsistencies, and perturbed controls. [Passing Linux smoke](evidence/linux-green/serial.log) · [Local checks](evidence/mac-tests.txt)

The [corrected request](request.json) records the operator's checkout paths and the pins sent to the controller. Native execution requires the controller's exclusive SETUP. The correction covers the recorder and its replay. Native CPU attribution remains pending.

## Replay

From the repository root:

```sh
packet=tools/durable-jobs-bench/results/2026-11-23-ordinary-native-cpu
(cd "$packet" && shasum -a 256 -c SHA256SUMS)
python3 -B "$packet/replay/check.py" -v
python3 -B "$packet/replay/offline.py" \
  tools/durable-jobs-bench/results/2026-11-23-ordinary-trusted-v1/evidence/ordinary-trusted-v1-dense \
  target/ordinary-cpu-archived-replay.json
```

On Linux, use the real staging helper and CPython:

```sh
packet=tools/durable-jobs-bench/results/2026-11-23-ordinary-native-cpu
stage=$(mktemp -d target/ordinary-cpu-clock.XXXXXX)
PYTHONPATH="$packet/replay" python3 -B -c \
  'from pathlib import Path; from diagnose import stage_worker; import sys; stage_worker(Path(sys.argv[1]), "on")' "$stage"
cp "$packet/replay/clock-smoke.py" "$stage/"
worker_sha=$(shasum -a 256 tools/durable-jobs-bench/worker.py | cut -d ' ' -f 1)
python3 -B "$stage/clock-smoke.py" "$worker_sha"
```

The macOS guest replay helper uses the pre-existing Linux queue proof VM, kernel and Python overlay. It builds a disposable initramfs in the output directory and removes it after the test. The retained run supplied ARM64 `libffi.so.8` from the Debian `libffi8_3.4.4-1_arm64.deb` package. [Debian package index](https://deb.debian.org/debian/pool/main/libf/libffi/) · [Guest input digests](evidence/linux-green/inputs.sha256)
