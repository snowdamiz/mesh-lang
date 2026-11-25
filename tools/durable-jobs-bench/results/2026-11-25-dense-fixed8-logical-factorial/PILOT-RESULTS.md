# Released dense fixed8 and logical-key pilot

Both same-image diagnostics and both reversed rotations reconciled on
Linode. The corrected reservation released with exit 0 at 03:50:25 UTC.
All 22 raw archives are listed with SHA-256 hashes in the replay. Each
measured arm completed 1,024 jobs with 2,048 independent application
write transactions. The stock and composed smokes both verified the
contract under the actual TLS worker role, including capacity contraction.
[Controller release](evidence/native-retry/controller) ·
[Pilot replay](evidence/native-retry/pilot-replay.json)

| Rotation | Clock | Composition | Saved vs. faster stock | Saved vs. better component |
| --- | --- | ---: | ---: | ---: |
| 1 | Submit | 976.610 ms | 344.571 ms | 194.680 ms |
| 1 | Drain | 816.264 ms | 346.880 ms | 160.303 ms |
| 2 | Submit | 991.201 ms | 374.213 ms | 188.994 ms |
| 2 | Drain | 830.131 ms | 367.072 ms | 186.335 ms |

The composition used 158 terminal XIDs in each rotation. Fixed8-only
used 182 and 180, and the fewer-XID stock duplicate used 870 and 875.
Last effect relative to enqueue commit was 808.050 and 819.316 ms.
Runner lifetime CPU was 518.524 and 536.966 CPU-ms, matched PostgreSQL
CPU was 1,130 CPU-ms in each rotation, and pinned host CPU was 1,730 and
1,760 CPU-ms. Each of those CPU values was below both stock controls in its
rotation. Both rotations passed all sealed advancement checks.
[Prospective thresholds](controller-packet-retry.json)

The first reservation stopped before timing after a successful native stock
smoke. The scorer expected the exported deployment slot count to equal the
per-runner count. The corrected retry checked 64 deployment seats plus the
existing 32 per runner and independently reconciled the original smoke.
The native adapters, control definitions, order and advancement gates
carried forward into the retry.
[Original diagnostic stop](evidence/native/README.md) ·
[Retry preflight](preflight-retry.json)
