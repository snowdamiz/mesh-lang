# Individual-terminal adapter SETUP on the Jobs proof ELF

The build-free SETUP released with exit 1 at 23:30:34 UTC. The
adapter and original ELF pins verified. Its first diagnostic used
the proof fixture, which hardcodes `event_log:true`. The exported
runner log has 1,008 lines with per-job events. Its shutdown report
shows a live logger with a sampled peak of 4,104 bytes.

The first diagnostic stopped in the canonical harness. The separate
Jobs correctness proof remains valid on its original instrumented
ELF. This adapter SETUP did not qualify a timing ELF and neither
timing window ran. A logger-off relink needs its own proof and release.

Inspect the [binding](../jobs-binding.json), [original release](controller-release.json),
and [failure summary](summary.json). Re-read the raw records with:

```bash
tar -xOzf raw.tar.gz ./0/failure.json
tar -xOzf raw.tar.gz ./0/runner-0.log | tail -1
```
