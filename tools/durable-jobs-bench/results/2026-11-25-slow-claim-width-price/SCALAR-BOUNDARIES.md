# Overlap between adjacent SLOW settlements

In the public 1×64 width-16 control, each of 512 jobs made two durable application writes and settled in its own transaction. For each adjacent pair in terminal commit order, the replay checks whether the second job had already committed its final application effect when the first job settled. This gives a necessary condition for the runner to have both exit-confirmed successes available by that instant. The archive lacks actor exits, request-send times and socket receipt times.

| Block | Next effect already committed, of 511 pairs | More than 1 ms earlier | More than 4 ms earlier | Mesh submit through settlement minus River | Net advance needed per counted pair to tie River |
|---|---:|---:|---:|---:|---:|
| 1 | 427 | 361 | 205 | +94.909 ms | 222 µs |
| 2 | 452 | 385 | 229 | +183.206 ms | 405 µs |
| 3 | 444 | 399 | 237 | −11.947 ms | Already ahead |
| 4 | 439 | 392 | 255 | −72.406 ms | Already ahead |
| 5 | 444 | 395 | 249 | +96.569 ms | 217.5 µs |

The last column divides the historical full-submit deficit by the first count. It assumes every counted boundary contributes equally and the advance transfers fully to final settlement. A pipeline can send later requests before earlier replies arrive while still requiring each job’s own transaction and acknowledgement. How often that is possible depends on confirmed exits and protocol state. A successor can also become ready between its predecessor’s database commit and client receipt, outside the counted opportunities. The archive establishes neither the actual number of sendable requests nor the saving per request.

The replay binds the ten raw archives to the published score, checks the independent application XIDs and required holds, and reconstructs each submit-to-final-settlement clock from the raw transaction and job ledgers. The 512 Mesh terminal commit times are distinct in every block. Run from this directory.

```sh
python3 -B scalar_boundaries.py | cmp - scalar-boundaries.json
```
