# Canonical activation path recovery

The original 05:34:31 UTC release and its A1 export are preserved at
`../evidence/activation`. It failed before the adapter launched the fixture.
The launcher, public ELF, input, scorer, qualification, activation arms and
both mirrored pilot groups are reused from the original seal.

The released 2×32 pilot staged each worker at `matrix/campaign/lane`. Stage
creation uses root's `umask 077` for directories and staged files. The worker
makes its lane, its immediate parent, `raw` and the attempt searchable
at root-owned `0711`, then creates a fresh root-owned `0700` adapter output.
The public launcher reads its own staged sources as root, copies its Python
entry into a separate root-owned `0711` temporary directory with a readable
file, gives only the empty adapter output to nobody:nogroup at `0700`, and
switches to nobody. The inventory leaves the worker's early `run_as` unset.

The failed 8×8 path put a phase between campaign and lane. Its worker
made the phase searchable but left the campaign root-owned `0700`.
Chowning the output leaf therefore could not permit the unprivileged lstat.
The recovery uses fresh `matrix/campaign-phase/lane` staging, with the phase
names retained in the first path component. The worker performs the same
ownership and chmod sequence as before. Each phase and arm has its own
attempt directory. Original files and failed output stay untouched.

`custody.json` records the filesystem-only replay on Linux, Python 3.12,
UID and GID 65534. The probe uses the actual stager with a local shell
transport and the actual worker entry with the benchmark subprocess replaced
by the launcher's switch-user sequence and a file probe. The original path
and a deliberately blocked recovered layout both fail at lstat. The released
2×32 layout and both recovered phase layouts allow a private `0600` file in
the `0700` adapter. The runner cannot list or write the root-owned ancestors
or read root-owned handoff files. A different UID cannot read the output.
No workload timing or benchmark binaries are involved. Reproduce using the
existing local Python 3.12 Linux image, isolated network, read-only checkout
mount and disposable container.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-dense-8x8/path-recovery
docker run --rm --network none --mount "type=bind,source=$PWD,target=/repo,readonly" \
    --workdir /repo --entrypoint python 3dc01f342e31 -B "$d/prove_custody.py" | cmp - "$d/custody.json"
python3 -B "$d/recovery.py" check
```

Only the sole controller enrolls the exact packet as
`2026-11-25-public-dense-8x8-path-recovery-activation.json` and
`2026-11-25-public-dense-8x8-path-recovery-pilot.json` in the existing
activation and pilot reservation. The recovery respects later bookings,
active grants and the prior 2×32 release. Dispatch from this checkout with
`python3 -B "$d/recovery.py" dispatch PHASE SHA --controller-root CONTROLLER_CHECKOUT`.
Put the controller release at `evidence/PHASE/controller-release.json`
inside the sibling and replay with `python3 -B "$d/recovery.py" replay PHASE`.
The original gated peer rule still requires both mirrored pilot groups to pass.
