# Authenticated recovery of the original file-session SETUP

The three-lock readback exported the original Linode output at 00:16:27
UTC on November 25. The archived SETUP-PASS marker was created inside
the original grant. Independent replay verifies the native transaction,
TLS and diagnostic proofs. The runtime was rebuilt to the exact public
archive digest. The original transport failure and empty export remain
in the [failure archive](../../setup-transport-failure).

The separate authenticated readback used strict two-space pin records.
It reverified the stock manifest and all seven members, the five
broker-source companions and sealed broker main, retained runtime and
broker, and the original source archive. The kernel, boot, disk and
all three locks matched. See the [pin receipt](member-pins-pre-release.txt)
and [summary](summary.json). These files preserve the recovered SETUP
evidence before its new controller release.

From the repository root, replay the complete export offline.

```bash
d=tools/durable-jobs-bench/results/2026-11-24-public-large-file-session/resume
PYTHONPATH="$d" python3 -B -c 'from controller import HERE, document, qualification, run; p=HERE/"evidence/native/setup-export-recovered"; assert qualification(p/"raw.tar.gz", document()) == run.read_json(p/"qualified.json")'
```
