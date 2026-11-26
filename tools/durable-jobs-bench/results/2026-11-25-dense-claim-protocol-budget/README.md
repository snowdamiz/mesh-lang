# Dense queue protocol intervals from the native statement logs

The repeated scalar settlement statement accounts for 58–78 ms of summed PostgreSQL Parse duration and 28–38 ms of Bind duration per 1,024 jobs in the four instrumented drains. Reusing a statement would still require Bind and execution. These sums span multiple backends and include their elapsed waits.

| Linode deployment and order | BEGIN to start of grant Parse, pairs | Interval union from log precision | End of grant Execute to start of COMMIT, pairs | Interval union from log precision |
| --- | ---: | ---: | ---: | ---: |
| Two runners, forward | 101 | 4.7–133.1 ms | 101 | 18.7–161.4 ms |
| Two runners, reverse | 102 | 4.7–133.8 ms | 101 | 25.4–157.4 ms |
| Eight runners, forward | 263 | 22.8–363.4 ms | 254 | 44.0–417.1 ms |
| Eight runners, reverse | 258 | 33.9–383.5 ms | 245 | 73.5–430.9 ms |

Each pair uses consecutive log records from one backend for the compiled public claim. The second interval includes the server reply, client validation and the next request. The replay restricts both logged events to the drain and unions overlapping intervals across backends. PostgreSQL logged event timestamps to milliseconds and operation durations to microseconds. The bounds allow for this precision. Logging overhead, scheduler delay, protocol processing and socket waits contribute to the intervals. They establish neither runner CPU nor recoverable wall time, and cannot price a change to the client commit gate. The two-runner point-key treatment used a different workload configuration and has no equivalent uninstrumented statement log.

Run `python3 -B replay.py | cmp - intervals.json` in this directory. Replay checks the four raw SHA-256 hashes against the [released trace score](../2026-11-24-public-dense-set-claim/NATIVE-RESULTS.md). No new workload is required.
