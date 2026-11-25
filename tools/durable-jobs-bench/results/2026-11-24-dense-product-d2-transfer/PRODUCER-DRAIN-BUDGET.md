# Dense producer and middle-drain budgets

These six 2×32 Linode rotations used 1,024 jobs, PostgreSQL 17.11 and the grouped-settlement public hot-pool executable. Times are paired differences in milliseconds. Positive gaps mean Mesh finished later. Each counterfactual holds every other interval in that arm fixed.

| Peer | Full-submit gap | Enqueue gap | Drain gap | Residual with free Mesh enqueue, range | Residual after matching the peer’s middle writes, range |
| --- | ---: | ---: | ---: | ---: | ---: |
| Oban | +19.9 | +14.3 | +5.6 | -350.7 to -104.2 | -234.2 to -33.1 |
| River | +336.2 | +83.4 | +246.3 | +129.5 to +202.8 | -267.5 to -209.2 |
| Apalis | +594.4 | +122.9 | +471.5 | +388.4 to +469.2 | +110.4 to +211.3 |

Free enqueue credits the full time from first submit to durable enqueue commit. Matching the peer’s middle writes replaces only the elapsed interval between its 103rd and 922nd durable second writes. The two counterfactuals are independent. Neither attributes a saving to a specific implementation. Medians of separate columns can come from different pairs.

| Rotation | Peer | Full-submit gap | Residual with free enqueue | Mesh middle minus peer middle | Residual after matching middle writes |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | Oban | +9.7 | -150.6 | +69.5 | -59.8 |
| 1 | River | +363.1 | +202.8 | +604.7 | -241.6 |
| 1 | Apalis | +618.0 | +457.7 | +424.1 | +193.8 |
| 2 | Oban | -23.7 | -178.3 | +91.1 | -114.9 |
| 2 | River | +310.6 | +156.1 | +577.7 | -267.1 |
| 2 | Apalis | +556.6 | +402.1 | +392.9 | +163.7 |
| 3 | Oban | +75.0 | -104.2 | +139.9 | -64.9 |
| 3 | River | +359.2 | +180.1 | +605.4 | -246.2 |
| 3 | Apalis | +648.4 | +469.2 | +437.1 | +211.3 |
| 4 | Oban | -192.2 | -350.7 | +42.0 | -234.2 |
| 4 | River | +288.0 | +129.5 | +538.8 | -250.8 |
| 4 | Apalis | +570.8 | +412.3 | +432.6 | +138.2 |
| 5 | Oban | +57.3 | -114.4 | +90.4 | -33.1 |
| 5 | River | +361.3 | +189.6 | +570.5 | -209.2 |
| 5 | Apalis | +631.8 | +460.1 | +425.2 | +206.6 |
| 6 | Oban | +30.2 | -128.3 | +67.5 | -37.4 |
| 6 | River | +313.1 | +154.7 | +580.6 | -267.5 |
| 6 | Apalis | +546.9 | +388.4 | +436.5 | +110.4 |

Run `python3 -B producer_drain_budget.py` in this directory. The underlying replay checks 72 raw archive digests, the two durable writes per job, and both clocks. [Original four-engine results](NATIVE-RESULTS.md)
