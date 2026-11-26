# Recovery of the canonical peer staging failure

The controller released the first peer grant at 05:53:51 UTC with exit 1.
Its stager was looking for the original stock inventory directory under the
path-recovery controller base. It stopped before the first arm. A1's staging
receipt, the failed release and both logs are preserved.

This peer-only intake makes the original inventory directory visible through
a relative link in the new controller base. The inherited stager and verifier
both use the original stock inventory bytes. Only staging and output paths
change. The new grant uses the original released pilot gate, ELF, launcher,
worker, scorer, 24-arm mirrored schedule and resource bounds.

`python3 -B peer_recovery.py check` verifies the sealed inputs.
`python3 -B test_staging.py` runs the actual host-free dispatch code with a
recording transport. The original base fails before dispatch. The new one
stages all five inventories, preserves the three original stock inventory byte
strings, and reaches every warmup and measured arm in the sealed order. The
transport records the invocations without launching workload processes.

The sole controller registers `controller-packet.json` once under
`2026-11-25-public-dense-8x8-peer-stage-recovery-peers.json`, as replacement
for the failed peer grant. Run `python3 -B peer_recovery.py dispatch SEAL
--controller-root CONTROLLER_CHECKOUT`. Publish the real controller release
under `evidence/peers/controller-release.json` and run
`python3 -B peer_recovery.py replay`. The activation and pilot do not rerun.
