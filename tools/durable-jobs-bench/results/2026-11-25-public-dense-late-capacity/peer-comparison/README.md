# Fresh canonical 8×8 peer comparison

Five fresh rotations compare late capacity on ELF `839c4f32eb4b`, two
same-image off controls, the qualified public grouped-hot `4924b588`
bridge, and the exact released 8×8 stock River, Oban and Apalis
inventories. Each rotation runs a warmup and a measured arm for every
lane in its sealed order. The complete replay requires 70 distinct raw
archives. The earlier peer archives qualify the parser and inventory
pins. The fresh comparison uses only its own timed arms.

The sole Linode controller opens one build-free grant after the existing
bookings. Its preflight verifies all 40 retained file pins, stock wrapper
access under the original launch identities, the pinned host and storage,
and the existing host noise rules. The stage path keeps the proven
two-level layout. The Mesh scorer binds exact ELF and manifest digests,
eight individual runners at eight seats each, two scheduler workers
per runner, 32 hot application sessions, and the physical cap of 96.
Each arm must reconcile the original 1,024 jobs, two independent committed
application writes per job and one producer transaction. Mesh also
reconciles each terminal cohort, its SQL and its owning runner.

The original peer comparison requires both clocks in the same direction
in every rotation beyond the larger of 5% of the peer clock and the
same-image duplicate spread. The original Mesh CPU envelope is the
higher off duplicate, independently for runner, PostgreSQL and host CPU.
The current bridge and the additional late-capacity gates are reported
per rotation. Replay retains paired full submit, drain, last durable
effect from enqueue and from submit, ranges, per-role metrics available
in the native captures, SQL counters and duplicate noise.

Run the offline packet and original stock parser checks from the checkout root.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-dense-late-capacity/peer-comparison
python3 -B "$d/compare.py" check
python3 -B "$d/test_offline.py"
```

The controller registers the exact `controller-packet.json` bytes before
opening its grant. It alone dispatches through its authenticated transport
and shared host lock. After release, rerun the native parser locally.

```sh
python3 -B "$d/compare.py" replay --output "$d/evidence"
```
