# Run the ordered enqueue qualification

Use the stock Linode and toolchain from the sealed 25-block baseline. The
matrix owner records the grant in
`target/linode-jobs-native/amd64-port-status.md`.
The grant has three fields after the marker. They are the reservation ID,
the UTC start, and the UTC end.

```text
NATIVE-ORDERED-ENQUEUE-SETUP <reservation-id> <UTC-start> <UTC-end>
```

1. From the repository root, verify the local bundle and print the planned
   stages. This command performs local reads.

   ```bash
   qualification=tools/durable-jobs-bench/results/2026-11-23-ordered-enqueue-qualification
   python3 -B "$qualification/replay/slot.py" --plan
   ```

2. During the granted SETUP interval, pass its exact line to the launcher.
   The launcher takes the local timing lock, remote active-arm lock, and
   native lock, in that order.

   ```bash
   status=target/linode-jobs-native/amd64-port-status.md
   grant="$(rg '^NATIVE-ORDERED-ENQUEUE-SETUP ' "$status" | tail -n 1)"
   python3 -B "$qualification/replay/slot.py" --grant "$grant"
   ```

   The build uses the sealed candidate source, an isolated Cargo directory,
   and the pinned stock compiler. It links a release runtime into the frozen
   fixture at optimization level 2. If the window ends before all stages
   finish, use the next explicit grant and pass the remaining stage names
   with `--steps`. Use the same `--build-id` when reusing a successful
   build. Use a new ID for a fresh build after a build failure.

3. Replay the native receipts. The replay checks the full worker ledgers,
   producer XIDs, ordinals, payloads, and receipt counts. It also checks the
   compiler fixture digests and carries forward each frozen peer entry
   from the baseline inventory.

   ```bash
   lane=target/linode-ordered-enqueue/native-qualification
   python3 -B "$qualification/replay/reduce.py" "$lane/native-runs" "$lane/reduced-native-v1"
   ```

The native producer comparison alternates stock and candidate with two
stock observations per group. Each arm gets a fresh clone of the same
database template. The replay computes PostgreSQL commit and producer
acknowledgment durations separately. The TLS relay runs in its own series.
The producer-only sparse run submits consecutive singletons. The separate
sparse worker diagnostic checks the frozen work and cadence.

The matrix owner runs any official repeat under its exclusive timing
reservation after accepting the completed handoff. The repeat reports
submit-to-settlement and commit-to-settlement separately for all five
profiles and the three frozen peers.
