# How much can faster enqueue change the native comparison?

Each column uses the Mesh and peer arms from the same rotation. The calculation holds each Mesh drain fixed at its observed time after enqueue commit. It asks whether faster submission alone could have matched the peer's full time from submission through its last settlement. A real enqueue change can also affect drain time.

The three counts partition the rotations. The required time saving is the signed difference between full Mesh and peer times, in rotation order. A negative value is the observed Mesh lead. All rows use single-transaction batches and include two independently committed application writes per job.

| Trusted Mesh workload | Peer | Already faster or tied | Faster enqueue could close gap | Faster drain also needed | Mesh enqueue, median ms | Required end-to-end saving by rotation, ms |
|---|---|---:|---:|---:|---:|---|
| Short | Oban | 4 | 1 | 0 | 73.8 | -193.5, -121.3, -166.3, -190.7, +32.5 |
| Short | River | 1 | 1 | 3 | 73.8 | -0.6, +148.1, +114.2, +9.4, +281.4 |
| Short | Apalis | 0 | 0 | 5 | 73.8 | +122.5, +146.4, +149.7, +130.1, +307.4 |
| Slow | Oban | 1 | 4 | 0 | 695.2 | +63.9, +68.9, +96.1, +56.3, -81.5 |
| Slow | River | 0 | 5 | 0 | 695.2 | +82.6, +311.3, +519.8, +367.6, +237.8 |
| Slow | Apalis | 0 | 3 | 2 | 695.2 | +614.9, +641.3, +755.8, +811.0, +457.7 |
| Large · EXTERNAL | Oban | 5 | 0 | 0 | 703.7 | -438.9, -456.9, -404.6, -702.0, -354.1 |
| Large · EXTERNAL | River | 5 | 0 | 0 | 703.7 | -1003.0, -1085.2, -1008.8, -1004.4, -973.1 |
| Large · EXTERNAL | Apalis | 0 | 5 | 0 | 703.7 | +495.5, +300.4, +502.9, +458.3, +485.5 |
| Dense | Oban | 0 | 0 | 5 | 561.4 | +1262.0, +1220.7, +1257.2, +1307.8, +1202.6 |
| Dense | River | 0 | 0 | 5 | 561.4 | +1736.2, +1583.4, +1529.0, +1640.8, +1842.9 |
| Dense | Apalis | 0 | 0 | 5 | 561.4 | +1959.7, +1909.6, +1923.5, +1955.0, +1975.4 |
| Large · EXTERNAL and 16 MiB work_mem | Oban | 5 | 0 | 0 | 769.0 | -385.3, -413.8, -402.3, -358.0, -420.0 |
| Large · EXTERNAL and 16 MiB work_mem | River | 5 | 0 | 0 | 769.0 | -935.5, -949.6, -1033.0, -974.3, -1176.8 |
| Large · EXTERNAL and 16 MiB work_mem | Apalis | 0 | 5 | 0 | 769.0 | +512.0, +507.4, +483.9, +524.3, +470.9 |
| Short · grouped settlements | Oban | 5 | 0 | 0 | 69.4 | -186.8, -168.4, -157.3, -133.0, -159.0 |
| Short · grouped settlements | River | 0 | 3 | 2 | 69.4 | +63.7, +77.8, +36.6, +18.5, +139.9 |
| Short · grouped settlements | Apalis | 0 | 0 | 5 | 69.4 | +107.1, +110.1, +166.7, +108.4, +179.8 |
| Slow · batch enqueue | Oban | 4 | 1 | 0 | 521.0 | +2.4, -156.6, -144.7, -123.2, -34.5 |
| Slow · batch enqueue | River | 1 | 4 | 0 | 521.0 | +202.1, +159.1, +184.7, -73.9, +202.7 |
| Slow · batch enqueue | Apalis | 0 | 3 | 2 | 521.0 | +187.2, +560.7, +533.4, +417.1, +536.0 |
| Short · batch enqueue and grouped settlements | Oban | 5 | 0 | 0 | 35.6 | -180.1, -208.6, -184.9, -198.2, -270.9 |
| Short · batch enqueue and grouped settlements | River | 2 | 2 | 1 | 35.6 | +29.5, +37.6, -9.9, +35.6, -15.2 |
| Short · batch enqueue and grouped settlements | Apalis | 0 | 2 | 3 | 35.6 | +3.1, +61.7, +30.2, +44.8, +69.1 |

Trusted Mesh runs handlers with application credentials in the shared process. The large series applies the same EXTERNAL payload storage policy on every engine. The grouped-settlement variant can commit up to eight successful terminal transitions together. [Matched runs and executable identities](scores.json) · [Interactive results](chart.html)
