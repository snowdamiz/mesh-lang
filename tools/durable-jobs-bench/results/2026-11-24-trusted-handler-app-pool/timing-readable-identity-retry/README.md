# Handler-pool reduced-UID helper access retry

The [previous raw warmup](../timing-identity-retry/evidence) stopped at
the new identity helper's file access. This retry stages the same
identity verifier in a fresh namespace. The root launcher exposes the
three Python helpers with mode 0644 and has the benchmark UID open
all three exact paths before the workload starts. It also checks the
root-owned launcher and worker. Replay requires the resulting access
receipt and the unchanged direct-child identity evidence.

The original qualified SETUP, four fixture ELFs, six rotations per
workload, and scoring gate are inherited from the earlier request.

```bash
python3 -B controller.py check
python3 -B controller.py replay
```
