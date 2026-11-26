# Two independent terminal connections on SLOW

The five released public SLOW rotations provide a separate price for the second terminal connection under investigation on SHORT16. At 64 seats, the simulation advances the final settlement by 31–47 ms even when it assigns zero cost to starting a second transaction, observing the handler exit, and receiving the server's acknowledgement. This falls short in all three rotations where the stock 1×64 runner trailed River from submit. A standalone SLOW transfer did not earn a booking.

| Rotation | Stock Mesh minus River submit (ms) | Two connections, zero added cost, submit saving (ms) | Two connections, with delays, submit saving (ms) |
|---|---:|---:|---:|
| 1 | +94.9 | 31.6–32.3 | 28.5–29.1 |
| 2 | +183.2 | 39.6 | 36.2 |
| 3 | −11.9 | 34.5–35.5 | 32.5–33.5 |
| 4 | −72.4 | 31.4 | 28.8 |
| 5 | +96.6 | 46.6 | 43.2 |

The rightmost column adds 250 µs after each second durable write for handler exit, 100 µs after each terminal commit for receipt, and 100 µs of extra service per candidate transaction. Ranges use the earliest and latest feasible grant times within the recorded millisecond. These values are model inputs. Native two-connection performance is unmeasured.

The replay preserves the original scalar send order, claim-stamp group sizes, delay from confirmed capacity to each grant, and the measured interval from that grant to each job's second write. It charges all time between a transaction becoming eligible and its observed commit to serial service, so the second lane receives credit for some work that may need the same dispatcher or CPU. Each replacement waits for sufficient individual terminal receipts. Every one-lane control reconstructs all 512 original terminal and effect timestamps. Real changes to job order, CPU contention, group commit or future grant widths could change the result.

Run `python3 -B terminal_lanes.py | cmp - terminal-lanes.json` in this directory. The replay verifies the five original Mesh and five River raw hashes and checks the 100 ms holds, 512 independent scalar terminal transactions per Mesh arm and 1,024 independent application transactions per arm. This price applies to the released SLOW scalar runner. The separate SHORT16 proof and the two held SLOW experiments have their own contracts.
