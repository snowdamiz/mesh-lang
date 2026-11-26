# Handler-pool fresh evidence directory failure

This attempt opened all three staged Python helpers as the benchmark
UID. The launcher then put a preflight receipt in the adapter's evidence
directory. The existing harness rejected that directory at its fresh
output check, before any workload. The controller released the slot
with exit 1 and the original SETUP remains qualified.

The [raw entry failure](entry-failure.txt) and [release](controller-release.json)
identify the pre-workload stop. Read the archived arm with:

```bash
tar -xOzf timing/01-noop/warmup-service-a/warmup-mesh/raw.tar.gz adapter.log status.json
```
