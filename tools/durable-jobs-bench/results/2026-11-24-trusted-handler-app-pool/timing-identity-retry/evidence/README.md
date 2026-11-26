# Handler-pool direct-child identity stage failure

The build-free retry released with exit 1 at 00:38:43 UTC. The first
service-control warmup stopped before launching its workload. The
reduced-UID process could open the original launcher helpers. It could
not open the newly staged `identity.py`, which retained the upload's
owner-only mode. The verified four-image SETUP remains available.

The [release](controller-release.json), [entry traceback](entry-failure.txt),
and original raw archive in
`timing/01-noop/warmup-service-a/warmup-mesh/raw.tar.gz` retain the attempt.

```bash
tar -xOzf timing/01-noop/warmup-service-a/warmup-mesh/raw.tar.gz adapter.log status.json
```
