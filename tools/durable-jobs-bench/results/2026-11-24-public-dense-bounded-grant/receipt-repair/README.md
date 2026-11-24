# Bounded-grant release receipt

The original intake handed the matrix owner a Python range of group numbers. Its JSON release writer raised a TypeError when serializing that range. This could lose the release record after the native process had already exited. Offline reproduction exercised the real receipt writer with its SSH probe stubbed and a temporary journal.

The intake wrapper verifies the original controller source digest and expands the range to a JSON array at the call to the matrix owner. The original schedule, native commands, executable, SQL, scoring and conditional peer gate all use the sealed source. The focused regression checks successful release records covering groups 0–6 and 7–12 and an idempotent second intake.

Run the local checks with `python3 -B intake.py check` and `python3 -B test_intake.py` from this directory. From the checkout root the sole controller can run the following command in the bounded-grant reservation.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-24-public-dense-bounded-grant/receipt-repair/intake.py intake cfc4c7221d086f0d00614ecd53f0222e2696abc0f5481ade6d5fc603a7ce7174
```

The contingent peer intake uses the same command with `intake-peers`. The original request and both schedules are byte-identical.
