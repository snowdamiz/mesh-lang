# Dense two-runner checkout and completion writer

The [native request](native-request.json) is for the sole Linode matrix controller. The new series follows the existing bookings. It depends on a qualified Linux D2 SETUP receipt from the [exit-confirmed D2 transfer](../2026-11-24-exit-confirmed-bulk-d2/controller-packet.json). The timing command consumes that exact sealed runner and performs no builds. No native result exists for this series yet.

## Native contrasts

- **Checkout.** Same Linux D2 ELF twice with the checkout switch unset, once with `MESH_PG_HOT_CHECKOUT=1`. The completion writer stays off. The SQL counter must show 2,048 application checkout probes per cold arm and fewer on D2. The replay reports every count.
- **Completion writer.** Same retained cold r2 ELF twice with the writer switch unset, once with `TRUSTED_COMPLETION_WRITER_MS=0`. Checkout stays cold. The replay checks and reports independent terminal XIDs and cohort widths separately from wall clocks.
- **Deployment bridge.** Native V1 uses 1,024 scalar enqueue calls in one durable enqueue transaction and individual terminal transactions. Both V2 contrasts use the same official bulk broker and migrator. They use eight bulk calls and zero scalar calls in one durable enqueue transaction. V2 admits work through the success-cohort policy and can settle up to eight successes per terminal XID even with the separate writer off. V1 and V2 timing measures combined deployment movement. The r2-off versus D2-off bridge measures a binary change with both checkout policies cold.
- **Peer comparison.** River, Oban, and Apalis use the existing pinned equal-topology 2×32 adapters. Each is measured in the same new rotations, using its frozen producer and settlement semantics. The earlier 8×8 stock peers and the controller-owned equal series retain their own evidence. [Existing native series](../2026-11-23-dense-matched-topology/README.md)

Both Mesh contrasts use two ordinary 32-slot runners, two fixed scheduler workers per runner, 16 warmed application TLS sessions per runner, PG 17.11 with a physical cap of 96, and the same frozen 1,024-job input. The pre-submit check records a TLS roster after the first launch and the additional 20 physical connections after the second. Both live runners and the broker are tied to their independent process identities and sealed executables. Both application writes per job commit under separate XIDs before durable terminal settlement.

The [preregistered plan](native-plan.json) has six rotations and a full warmup and measured arm for each lane. For each contrast, the candidate must beat both offs on both submit-to-terminal and commit-to-terminal by more than the off-control spread and 5% of the faster off. The final application effect must also commit earlier than both offs. At least four of six fully qualified rotations must clear that rule. A probe or terminal-XID reduction is reported independently. This experiment authorizes no writer promotion.

The controller must replay the retained r2 native fault and teardown receipts for both writer modes and qualify the Linux D2 transfer, including its stale session and lost application, claim, and terminal receipts. The controller checks the native host, boot, device, CPU mask, artifact pins, and three timing fences. Rotations 1 and 2 must qualify before 3–6 can start. The dispatcher stores warmups, measured arms, worker/SQL observations, full redacted raw archives, and interrupted or failed attempts under `evidence/native/`.

## Offline checks

Run from the checkout root with Python 3.11 or newer.

```sh
p=tools/durable-jobs-bench/results/2026-11-24-dense-two-runner-checkout
python3 -B "$p/native-controller.py" check
(cd "$p" && shasum -a 256 -c SHA256SUMS)
```

The controller timing entry point refuses to run without the exact active controller grant and qualified candidate receipt. Its replay command uses the archived receipts and does not contact the host.

## Interrupted workstation diagnostics

The [workstation archive](interrupted-workstation.tar.gz) retains two failed pre-lock checks and one cold D2 smoke interrupted during drain. It contains no qualified arm or scored rotation. PostgreSQL shut down cleanly, both owned runners exited, the disposable cluster and scratch were removed, and the shared timing lock was released. The archive includes the preflight source, raw partial receipts, and an internal SHA-256 manifest. The resolved first-arm pre-submit evidence showed both 32-slot runners and all 41 TLS sessions. No probe or wall-clock inference is drawn from this partial arm.

The earlier workstation writer comparison and original native V1 product measurements remain in their [original packet](../2026-11-24-dense-two-runner-completion/README.md).
