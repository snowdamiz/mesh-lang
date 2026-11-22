# Reproduce the Linux Jobs regression screens

Run from the repository root on macOS ARM64. The launcher uses the
recovered Linux kernel and the isolated GNU ARM64 PostgreSQL 17.11
guest base from the adjacent `t3-45996d9a` checkout. It uses the
native OTP tree, kernel and Oban BEAM build in
`target/linux-queue-perf`. The replay uses Elixir 1.20.4 from
Homebrew. Packaging and timing take `target/queue-performance.lock`
for one pair at a time.

Use a release ELF linked with the frozen named-effects fixture. The
[frozen replay archive](../linux-warm-replay-20261122.tar.gz) contains
its Mesh source, the runtime build and link commands, and the Oban
producer source. The fixture builds against Rust 1.99.0, Zig 0.15.2,
and the historical compiler at `8526a5bd`. The archived benchmark
scripts use the two-role observer recorded in the paired receipt.
This driver uses the corrected observer at `7b61e9bb`. It reads
`oban_erl_flags` from the frozen profile and records each Oban
runner's effective emulator settings. The older two-scheduler
diagnostic driver is available at `229ce02d`.

The retained ELF has SHA-256
`fe719b77c033f0c08e61138291eb119487c796476e23fe9fb544711dd50233c8`.
Its release runtime archive has SHA-256
`f99dbbdf5cb0fd53bf85aa9609dfce335d5fada770ae3217e24b02ec818ae4e6`.
The replay link used the runtime source at `d8741576`, which matches
the runtime after `35fa53ff`.

```sh
work=target/jobs-perf-work
mesh="$work/mesh-frozen-final"
digest=fe719b77c033f0c08e61138291eb119487c796476e23fe9fb544711dd50233c8
for profile in noop slow sparse short large; do
  python3 -B proof/jobs-parity/linux-regression/run.py "$profile" \
    "$work/canonical/${profile}-replay-01" --mesh "$mesh" --mesh-sha256 "$digest"
done
```

Use a fresh output directory for each run. For an alternating pair
pass `--oban-first`. Use `summarize.py` with the successful output
directories to verify the exported job ledgers and reproduce the compact
receipt. The driver verifies the exact input hash for
each profile and rejects missing jobs, wrong payloads or mismatched
write identities. It verifies two ordered committed writes per job
and counts executing and terminal jobs using the same
sampling interval for both arms. The guest applies the same physical
and per-database caps, with zero reserved connections. The producer
and observer share one `parity_producer` connection in both arms.
The statement counters report its combined traffic under that role.
They report `compiled_caller` traffic separately from Jobs runner
SQL. The throughput, pickup and `wait4` runner-tree CPU definitions
are the same as in the parity driver.

| Profile | Runners and slots | Guest | Physical cap | Input |
| --- | --- | --- | ---: | --- |
| No-op | 8 runners, 64 slots | 2 vCPUs, 2 GiB | 96 | 1,024 jobs, 1 KiB, 0 ms hold |
| Slow | 1 runner, 64 slots | 2 vCPUs, 2 GiB | 68 | 512 jobs, 16 KiB, 100 ms hold |
| Sparse | 1 runner, 1 slot | 2 vCPUs, 2 GiB | 5 | 24 one-job batches, 1 KiB, 1 ms hold |
| Short | 1 runner, 16 slots | 2 vCPUs, 2 GiB | 20 | 128 jobs, 1 KiB, 1 ms hold |
| Large | 8 runners, 64 slots | 2 vCPUs, 4 GiB | 96 | 64 jobs, 512 KiB, 1 ms hold |

No-op, slow and large run a full unmeasured batch for each engine
before the comparison. The inherited base image, reconstructed native OTP
build and observer adapter distinguish these screens from the old
guest receipts.

Run `python3 -B proof/jobs-parity/linux-regression/score.py RECEIPT.json`
on the combined receipt. The scorer verifies five pairs per profile,
one release ELF and identical common component hashes. It uses
the existing parity limits for throughput, pickup, CPU, RSS,
connections, processes, threads and the sparse listener.
