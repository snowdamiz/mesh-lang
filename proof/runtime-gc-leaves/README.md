# Actor GC skips pointer-free text and byte bodies

The runtime baseline is `f8f8ed21` at the proof checkout `6c5ca318`.
The sparse and short queue receipt landed before integration without
changing the compiler or runtime baseline.

Actor GC marks reachable leaves, including those reached through an interior
pointer or a loan. It skips their bodies during tracing. The collector still
uses the 16-byte header, the 256 KiB pressure floor, and the same sweeping
and zeroing policy. A reused block takes the new allocation's flags. The
global arena uses its existing allocator.

The leaf flag comes from runtime constructors for text and selected Bytes
values. Native ABI 1 exposes String and Bytes as opaque values and borrows
pointer arguments only for the call. The raw `mesh_string_new`,
`mesh_bytes_new`, and `mesh_bytes_copy_from` constructors still use the
conservative allocator. Mutable byte builders, arbitrary native
`mesh_gc_alloc_actor` storage, message copy materialization, and byte copies
that can carry raw native data also use the conservative allocator.
Collectors still trace incoming roots, stack and register words, loaned
roots, and the bodies of generic allocations.

## Ordinary release programs

The Linux AArch64 guest ran five independently compiled Mesh programs. Each
pair shares a Mesh object compiled at optimization level 2. Both archives
use the standard GC policy and the programs ran with one scheduler worker.
The harness timed whole-process user and system CPU and checked the output
against an independent oracle. It ran six interleaved rotations per boot,
with two copies of the parent and one candidate per rotation. The second boot
reversed the arm order and the rotation order. Values are medians with
minimum and maximum in brackets.

| Fixed work | Boot 1 parent → candidate, CPU ms | Boot 2 parent → candidate, CPU ms |
| --- | ---: | ---: |
| 4,000 UTF-8 strings of about 512 KiB, up to 12 live | 294.7 [207.3–434.2] → 128.4 [70.5–262.9] | 952.2 [723.9–1,263.4] → 125.8 [104.9–181.3] |
| 4,000 Bytes values of 512 KiB, up to 12 live | 190.1 [171.5–223.7] → 59.3 [55.2–220.7] | 849.1 [538.0–1,080.3] → 183.2 [70.3–413.1] |
| 1,600 parses and encodes of string-rich JSON | 482.9 [441.1–812.4] → 461.1 [361.0–749.4] | 1,987.8 [774.9–3,612.0] → 1,613.4 [781.5–2,037.4] |
| 1,000,000 small strings with a bounded live list | 85.1 [72.7–364.9] → 87.7 [80.2–234.4] | 367.3 [94.4–801.5] → 186.0 [116.6–894.6] |
| 350,000 mostly pointer-bearing nested list updates | 144.5 [71.6–382.5] → 90.2 [78.1–264.9] | 741.1 [255.0–3,123.8] → 819.1 [494.3–1,095.6] |

Each iteration checks values retained from an earlier iteration. UTF-8
improved in all 12 rotations against the median of that rotation's two
parent runs. Bytes improved in 11. CPU measurements for JSON, small
strings, and nested lists varied within the controls and between boots.
Those measurements leave their performance effect unresolved.

The separate scan attribution run counted 57 collections and about 65.9
million payload words for each 1,000-iteration UTF-8 and Bytes program.
The small-string and nested-list controls spent 0.183 ms and 3.110 ms
respectively in transitive scans over 100,000 iterations. The majority of
the nested-list scanning was through reference-bearing bodies.

## Compiled queue attribution

The unarranged ELF reproduced the frozen parent SHA-256
`32ceecf560ef70c5f1443d0688690a77a206070a897156a58b7ba321cd2f3070`.
The candidate used the same app object, native archive, and linker inputs
with the candidate runtime archive. Its ELF SHA-256 is
`21c403a51c5060caffce782386a7cc766537829f6b53735bbcef24ac0cab87ae`.
All 1,219 non-runtime archive members matched byte for byte.

The standard-policy attribution binary scanned all reached bodies and used
the thread CPU clock inside the transitive traversal. Before the one durable
enqueue commit, the producer ran seven collections and scanned 30,086,405
string words, or 240,691,240 bytes. Transitive scanning used 19.757 ms
against 576.339 ms of producer CPU through the reply. After commit, the
64 verified worker PIDs ran 128 collections and scanned 8,394,112 string
words, or 67,152,896 bytes. Their transitive scans used 5.115 ms of thread
CPU, about 0.14% of runner and child CPU through shutdown. The producer ran
no collections after commit. The workers ran no collections before commit.
The runner processes recorded no collections. The worker cost was too small
to warrant an Oban score.

A separate strace run used the actual candidate ELF. The trace matched
every SQL-recorded worker PID to one successful exec, one exit with status
zero, and one successful consuming wait. The workload also reconciled 64
enqueue receipts, claims, attempts, and settlements, plus two independent
application writes per job and the expected session resets.

## Compiled queue release rotations

The frozen input has 64 jobs with 512 KiB arguments. Both rotations use the
unarranged release ELFs and the same job fixture. The submit-to-commit
window includes producer input encoding and PostgreSQL work. The
settlement window starts at that durable commit timestamp and ends at
the last durable settlement.

| Rotation | Submit to commit, seconds in run order | Commit to last settlement, seconds in run order |
| --- | --- | --- |
| Parent, parent, candidate | 1.276, 1.884, 2.401 | 0.345, 1.023, 2.727 |
| Candidate, parent, parent | 2.268, 3.442, 2.300 | 0.353, 0.533, 1.468 |

Every measured arm reconciled 64 jobs and one enqueue transaction.
Producer CPU through the reply was 252.0, 304.4, and 365.1 ms in the
forward rotation. It was 357.0, 697.4, and 389.3 ms in the reverse
rotation. The change leaves encoding and work submission in the
original path. The two rotations give opposite timing results under
large control variation. The queue timing result is inconclusive.

## Verification

All 21 individual checks passed on the hashed candidate files. They include
the workspace build and suites, compiled C FFI and actor GC-stress tests,
policy laws, pinned Z3, and PostgreSQL 17.11, 18.7, and 19.0 compatibility and
effect-fence proofs. The workspace wrapper exited after its final child
check passed. The PostgreSQL series ran in a fresh wrapper.

[Machine-readable receipt](receipt.json)

## Replay

The local scripts, link manifests, raw samples, gate logs, and retained
archives are in `target/general-gc-string-leaves`. Run the commands in
`target/general-gc-string-leaves/REPLAY.md` from the checkout root. The
replay uses the guest assets pinned by
[the integrated uncut proof](../jobs-parity/linux-integrated-uncut/README.md).
