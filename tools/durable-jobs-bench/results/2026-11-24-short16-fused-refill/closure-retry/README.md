# Short16 fused-refill closure and eight-lane amendment

This amendment is build-free. It does not change the sealed request
(`0c296720…`), source archive (`c4c5cc22…`), ELF or seven-cut SETUP. The controller
found two defects in the sealed packet before TIMING. Both reproduce offline on the
actual inputs:

1. **Adapter.** `replay/backend.py` recompiles the frozen `arm.Cluster.reconcile_trial`
   outside its class body. Its zero-argument `super()` then raises
   `super(): __class__ cell not found`. `backend.py` here makes the same change that
   qualified natively for the public product in
   `2026-11-24-trusted-jobs-product-composition/closure-retry`: it reads the original
   `__class__` cell, asserts that it is `arm.Cluster`, and calls
   `super(arm.Cluster, self)`. No other line changes.
2. **Driver.** The inherited `interleave-v2.validate` accepts at most six lanes, but
   the frozen plan has eight. `controller-timing.py` here runs the unchanged sealed
   driver (pinned with its `intake.py` and `interleave-v2.py`) with these pinned
   substitutions only:
   - The lane-count bound becomes "exactly the frozen eight short16 lanes".
   - The TIMING grant is `2026-11-24-short16-fused-refill-closure-retry`.
   - The overlay SETUP release is required.
   - Every bound mesh lane must use a `replay-closure-retry` adapter.

`launcher.py` is the corrected form of the controller's
`short16-fused-refill-slots.py`. It loads that file unchanged (pinned) and reuses its
intake, original SETUP, the public short-smaller dependency, `public-control.json`,
the original binding, replay and pilot gate. It adds three steps:

- `setup.sh` installs the sibling `replay-closure-retry/` after the original SETUP
  qualifies. It then runs one real native short diagnostic ledger each for
  `product-group` and `product-fused`. Release: `…-closure-retry.released.json`.
- `rebind()` moves only the four adapter pins and the launcher path of each mesh
  lane. The candidate lanes go to the new overlay. `public-group` goes to the public
  overlay that has already qualified natively (`590a3dc9…` qualification).
- TIMING runs through the amended driver. Groups 1–2 run first. Groups 3–5 run only
  under the unchanged sealed gate: every paired fused/off row must exceed 0 on both
  clocks.

## Offline proof

`offline/` holds the proof: `offline-proof.json`, its log, and the original and
rebound inventories. It ran as uid 501 with SSH and locks stubbed to fail. It used
the exact frozen harness (`arm.py` `d9f984da…`) and the real public manifest
`979cd601…`. The candidate manifest is synthetic until the native SETUP exports one.

| Check | Original | Corrected |
| --- | --- | --- |
| `reconcile_trial` | `RuntimeError: super(): __class__ cell not found` | reaches `native_regression.Cluster` |
| Pilot driver, groups 1–2 | `ValueError: specify between two and six lanes` | validates the eight lanes, exit 0 |
| Amended driver on the un-rebound inventory | — | rejected: not bound to the closure-retry adapter |
| Groups 3–5 before the pilot reconciles | — | rejected: reconcile groups 1 and 2 first |
| Seven lanes, or a renamed lane | — | rejected: use exactly the frozen eight lanes |

`offline-01-unlabelled-assert/` keeps the first run. It was identical except that the
rebinding assertion had no message.
