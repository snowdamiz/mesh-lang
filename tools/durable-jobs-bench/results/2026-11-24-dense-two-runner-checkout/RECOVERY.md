# Dense native matrix import recovery

The [original first warmup](evidence/native/blocks/01-noop.1795492030327231000/warmup-d2-off-a/warmup-mesh/raw.tar.gz) exited 1 at `from identity import …` before a workload or ledger. Its SHA-256 is `cb31d68460ad2d66e4b4b48459d8198a2ad3fb0a86eba137d6f6d853b1f7bc5f`. The original native grant, release, staging inventory and failure remain in the portable archive.

The sole controller submitted [the build-free import retry](request-import-retry.json) with seal `545f2843310221b6775dd6f5e67fd84c87eba5fe47d57c0e567541b4c1aea18a`. Its [launcher](native-dense-import-retry.sh) directly loads the pinned sibling `identity.py` and then enters the original arm. The [Linux probe receipt](evidence/import-proof/linux.json) records UID/GID 65534, zero effective capabilities, root-owned mode 0711 directories and mode 0644 files. Directory enumeration failed while an exact-path read succeeded. The original entry failed at import. The controller retry reached the deliberate one-CPU safety guard before PostgreSQL, so this local probe ran no workload. The controller's native warmup receipts qualify the actual adapter path.

The retry used the same recovered D2 ELF, cold r2 and V1 images, official bulk broker, three 2×32 peer lanes, six frozen rotations, warmup and measured phases, affinity and grant fences, and predeclared advancement rules. The controller released rotations 1–2 and 3–6 successfully under separate grants. All 120 raw retry arms reconciled, including 60 measured arms.

D2 met the advancement rule in four of six rotations. The measured D2-on arms made four application checkout probes each, down from 2,048 per cold arm. Median paired elapsed savings against the mean of the two cold controls were 158 ms from submit and 141 ms from durable enqueue. Cold r2 zero-hold reduced terminal XIDs in all six rotations and met the wall-clock advancement rule in zero. [Verified native results](evidence/dense-native-replay.json)

## Portable replay

The [native archive](evidence/dense-native-portable.tar.gz) includes the original failure and controller release, the recovered D2 qualification, both intake packets, the execution inventories, retry grants and available releases, and the exported raw retry attempts. Its internal SHA-256 manifest covers all included files. The replay reconstructs each scored rotation from raw ledgers, verifies the unchanged plan and launch provenance, and labels incomplete rotations. It uses Python 3.11 or newer and denies host access.

```sh
mkdir extracted
tar -xzf dense-native-portable.tar.gz -C extracted
python3 -B extracted/replay-native.py replay-tree extracted
```
