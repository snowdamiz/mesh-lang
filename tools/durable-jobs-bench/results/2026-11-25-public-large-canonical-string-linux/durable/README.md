# Parked public LARGE FILE comparison

The FILE producer trial requires a successful release and raw replay of the public component screen. Only the sole authenticated capacity driver may activate either phase. The product packet is separate from the component packet and reuses its suffix and earned result hash. Its source study is producer-only.

1. FILE SETUP links two uninstrumented producer ELFs with the original public compiler, FILE session source and fixture libraries. The stock relink must use the exact original stock runtime archive hash. The candidate relink uses the sealed public candidate patch. Diagnostic runs use the original broker plus both relinks, with the original runner, migrator, EXTERNAL adapter, public fixture and 15-second broker reply watchdog.
2. FILE TIMING requires the SETUP release and export on top of the component release. Each of five blocks contains a warmup and measured trial on all four lanes. The duplicate controls share the stock relink ELF. A separate lane uses the exact released broker. The candidate changes only the producer ELF. All lanes retain 64 × 512 KiB payloads, eight runners with eight slots each, CPUs 0 and 1, the original 96-connection cap, 4 MB work memory, the EXTERNAL policy and PostgreSQL durability. The original scorer checks launch pins, scalar writer boundaries, process identity, TLS and durable ledgers. The replay reports paired differences to the faster duplicate and the exact broker for enqueue, settlement and post-COMMIT drain. The duplicate spread stays visible in each block and at the median.

The full FILE trial has 40 arms. Peer releases remain independent. This package does not claim a product saving or release readiness.

## Commands for the sole controller

The [packet](controller-packet.json) records the exact argv and capsule hash. In particular, the FILE link stage takes its small link capsule on stdin. It requires the native screen summary at the digest returned by the offline screen replay. Capture qualification stdout and the returned summary digest. Export while fenced, bind that digest as `qualified_sha256`, and bind the raw archive digest as `raw_sha256`. Preserve failures with their original logs and raw export. From the repository root:

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-large-canonical-string-linux
python3 -B "$d/durable/pack-file.py" | shasum -a 256
python3 -B "$d/durable/file.py" timing qualification-dir screen-dir file-link-dir file-timing-dir
python3 -B "$d/durable/file.py" replay qualification-dir screen-dir file-link-dir file-timing-dir
```

The two earlier directories follow the component packet. `file-link-dir` holds `controller-release.json` and the FILE SETUP `raw.tar.gz`. The release binds the FILE packet, component screen digest, qualification digest and original suffix. The last directory holds the FILE timing release, grant and exact matrix output. The timing subcommand requires the registered FILE packet and an active TIMING grant before any host work. Raw replay rereads all warmup and measured arms.
