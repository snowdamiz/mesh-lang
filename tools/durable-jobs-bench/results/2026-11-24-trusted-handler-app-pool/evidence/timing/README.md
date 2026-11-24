# Handler-pool first timing grant

The Linode controller released groups 1–2 with exit 1 at 23:16:31 UTC.
The first dense service-control warmup had two ready runners. Its
identity hook then failed with EACCES reading one child's
`/proc/PID/exe`, and the harness shut down before jobs were submitted.
No measured arm started. The four-ELF SETUP remains qualified.

See the [failure signature](failure.json), [release](controller-release.json),
and raw export in `01-noop/warmup-service-a/warmup-mesh/raw.tar.gz`.

```bash
tar -xOzf 01-noop/warmup-service-a/warmup-mesh/raw.tar.gz adapter/failure.json status.json
```
