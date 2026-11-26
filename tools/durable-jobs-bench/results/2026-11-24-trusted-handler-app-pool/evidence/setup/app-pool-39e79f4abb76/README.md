# Qualified native handler-pool fixtures

The Linode SETUP retry released with exit 0 on 2026-11-24.
The raw export has the pinned AMD64 host and toolchain identity,
the four linked fixture manifests, and the compiled two-runner
`APP_POOL_SCOPE_OK` witness. Replay verifies a common compiler and
runtime, four distinct runner ELFs, and the fixture and SQL digests.

From the repository root, verify the archived raw evidence.

```bash
python3 -B tools/durable-jobs-bench/results/2026-11-24-trusted-handler-app-pool/replay/controller.py verify-setup
```

See the [controller release](controller-release.json), [raw export](raw.tar.gz),
and [qualification](../../native-qualification.json).
