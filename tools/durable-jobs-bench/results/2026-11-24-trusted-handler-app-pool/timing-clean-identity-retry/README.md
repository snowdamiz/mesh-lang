# Handler-pool staged helper check with fresh output

The [previous attempt](../timing-readable-identity-retry/evidence) passed
the staged helper opens and then stopped at the untouched harness's fresh
evidence directory check.

The repaired launcher sends the helper-access receipt to the worker's
existing adapter log. The workload evidence directory remains empty
at the harness entry. The same reduced UID still opens each helper;
replay requires the marker and the original direct-child identity checks.

```bash
python3 -B controller.py check
python3 -B controller.py replay
```
