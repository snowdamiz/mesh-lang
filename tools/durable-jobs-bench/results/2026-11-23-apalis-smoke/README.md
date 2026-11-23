# Replay the November 23 Apalis SETUP evidence

From the checkout root, run the offline verifier and its negative controls with
Python 3.11 or newer.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-23-apalis-smoke/replay.py
python3 -B tools/durable-jobs-bench/results/2026-11-23-apalis-smoke/test_replay.py -v
```

Replay needs no running database, Rust toolchain, or network access. It checks
the original archives and their member hashes against the native receipt. It
matches the staged adapter and fixture hashes against this checkout and uses
`staged-verify.py` for the exact shared verifier deployed with the adapter.
It reproduces the saved sparse reconciliation and checks the outcome with the
checkout's shared verifier.

The Linux AMD64 run used Rust 1.99.0 and Apalis and apalis-postgres 1.2.0.
The deployed release manifest and both arms record executable SHA-256
`b3c2fb2b472d7c8b2706e13c38281a46958b669f9d4cf22ca252636d083e7e66`.
The archive records the executable and PostgreSQL installation digests as
captured on the host. The binaries themselves are outside the archive.
PostgreSQL 17.11 appears in both the startup log and the application
connection's observation of `server_version_num`.
[Native receipt](../2026-11-23-native/apalis.json)

| Original archive | Stored bytes | Expected outcome |
| --- | ---: | --- |
| `functional-v1.tar.gz` | 22,451 | One rollback, two completions including one retry, and one exhausted failure. The raw export has three product rows, four attempts, and three effects. |
| `sparse-trace-off-v1.tar.gz` | 33,089 | 24 singleton enqueue calls and 24 completions. The raw export has 24 product rows, 24 attempts, 24 effects, and 48 distinct application XIDs. The sampled connection peak is four, with a cap of five. |

These are SETUP correctness runs. Each archive contains the exact native
command in `command.txt`. To print the commands without extracting files,
run the following from the checkout root.

```sh
evidence=tools/durable-jobs-bench/results/2026-11-23-apalis-smoke
tar -xOzf "$evidence/functional-v1.tar.gz" command.txt
tar -xOzf "$evidence/sparse-trace-off-v1.tar.gz" command.txt
```

To build and exercise both profiles with a disposable PostgreSQL cluster on
your machine, follow the adapter's build and run instructions.
[Build and run Apalis](../../rust/README.md)
