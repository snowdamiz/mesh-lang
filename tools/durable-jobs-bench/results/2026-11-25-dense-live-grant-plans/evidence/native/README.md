# Live public grant-plan diagnostic releases

The Linux SETUP compiled and loaded the pinned official PG17.11
module and passed the nested query-ID canary. SETUP released with
exit 0 at 01:42:20 UTC. The timing slot stopped on its first off A
arm and released with exit 1 at 01:42:35 UTC. The reduced-UID
observer received EACCES opening one runner's `/proc/PID/fd`
directory. No matched plan-sampling rotation qualified.

See the [SETUP release](setup-controller-release.json),
[module and canary receipt](setup.json), [TIMING release](controller-release.json),
and [first raw failure](failure.json). The fail-fast packet calls
for one attempt.

```bash
tar -xOzf blocks/01-noop/measured-off-a/measured-mesh/raw.tar.gz adapter/failure.json
```
