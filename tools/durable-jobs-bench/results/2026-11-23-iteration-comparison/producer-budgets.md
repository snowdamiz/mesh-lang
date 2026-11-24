# How much can faster enqueue change the native comparison?

Each column uses the Mesh and peer arms from the same rotation. The calculation holds each Mesh drain fixed at its observed time after enqueue commit. It asks whether faster submission alone could have matched the peer's full time from submission through its last settlement. A real enqueue change can also affect drain time.

The three counts partition the rotations. The required time saving is the signed difference between full Mesh and peer times, in rotation order. A negative value is the observed Mesh lead. All rows use single-transaction batches and include two independently committed application writes per job.

The minimum drain saving credits Mesh with free enqueue in each pair. A zero means the observed Mesh drain already fits inside its peer's full submission window. Sparse jobs have separate submissions and are shown in the per-job response tables.

| Trusted Mesh workload | Peer | Already faster or tied | Faster enqueue could close gap | Faster drain also needed | Mesh enqueue, median ms | Minimum drain saving with free enqueue, median ms | Required end-to-end saving by rotation, ms |
|---|---|---:|---:|---:|---:|---:|---|
| Public short | Oban | 5 | 0 | 0 | 42.6 | 0.0 | -244.9, -236.8, -205.4, -256.1, -198.1 |
| Public short | River | 3 | 1 | 1 | 42.6 | 0.0 | -21.8, +48.5, -7.0, +42.0, -40.9 |
| Public short | Apalis | 0 | 0 | 5 | 42.6 | 26.7 | +77.8, +76.1, +48.5, +64.8, +85.1 |
| Public dense · all 8 × 8 | Oban | 0 | 0 | 5 | 173.4 | 363.7 | +1122.3, +543.4, +535.5, +530.1, +613.2 |
| Public dense · all 8 × 8 | River | 0 | 0 | 5 | 173.4 | 781.9 | +1599.7, +961.6, +904.8, +904.0, +1046.5 |
| Public dense · all 8 × 8 | Apalis | 1 | 0 | 4 | 173.4 | 1091.1 | +1858.3, -24669.0, +1215.7, +1264.5, +1281.0 |
| Public dense · Mesh 2 × 32, peers 8 × 8 | Oban | 5 | 0 | 0 | 175.5 | 0.0 | -113.4, -234.9, -151.9, -83.2, -100.2 |
| Public dense · Mesh 2 × 32, peers 8 × 8 | River | 0 | 0 | 5 | 175.5 | 114.3 | +289.8, +265.8, +388.3, +233.8, +338.4 |
| Public dense · Mesh 2 × 32, peers 8 × 8 | Apalis | 0 | 0 | 5 | 175.5 | 403.3 | +575.7, +557.8, +581.5, +614.0, +664.8 |
| Public dense · all 2 × 32 | Oban | 0 | 4 | 1 | 162.2 | 0.0 | +107.7, +181.9, +160.8, +161.1, +137.9 |
| Public dense · all 2 × 32 | River | 0 | 0 | 5 | 162.2 | 304.2 | +441.1, +504.1, +562.1, +469.9, +401.6 |
| Public dense · all 2 × 32 | Apalis | 0 | 0 | 5 | 162.2 | 579.1 | +735.7, +807.5, +772.2, +719.8, +731.6 |
| Public slow · all 1 × 64 | Oban | 5 | 0 | 0 | 530.5 | 0.0 | -259.6, -128.7, -133.7, -188.2, -247.0 |
| Public slow · all 1 × 64 | River | 2 | 3 | 0 | 530.5 | 0.0 | +94.9, +183.2, -11.9, -72.4, +96.6 |
| Public slow · all 1 × 64 | Apalis | 0 | 4 | 1 | 530.5 | 0.0 | +416.8, +509.1, +568.0, +508.7, +382.0 |
| Public large · default storage | Oban | 4 | 1 | 0 | 1814.3 | 0.0 | -62.7, -86.1, +129.9, -175.5, -14.8 |
| Public large · default storage | River | 5 | 0 | 0 | 1814.3 | 0.0 | -569.4, -568.6, -413.5, -674.8, -718.7 |
| Public large · default storage | Apalis | 0 | 5 | 0 | 1814.3 | 0.0 | +718.7, +830.8, +1034.6, +770.0, +829.6 |
| Short | Oban | 4 | 1 | 0 | 73.8 | 0.0 | -193.5, -121.3, -166.3, -190.7, +32.5 |
| Short | River | 1 | 1 | 3 | 73.8 | 40.5 | -0.6, +148.1, +114.2, +9.4, +281.4 |
| Short | Apalis | 0 | 0 | 5 | 73.8 | 61.6 | +122.5, +146.4, +149.7, +130.1, +307.4 |
| Slow | Oban | 1 | 4 | 0 | 695.2 | 0.0 | +63.9, +68.9, +96.1, +56.3, -81.5 |
| Slow | River | 0 | 5 | 0 | 695.2 | 0.0 | +82.6, +311.3, +519.8, +367.6, +237.8 |
| Slow | Apalis | 0 | 3 | 2 | 695.2 | 0.0 | +614.9, +641.3, +755.8, +811.0, +457.7 |
| Large · EXTERNAL | Oban | 5 | 0 | 0 | 703.7 | 0.0 | -438.9, -456.9, -404.6, -702.0, -354.1 |
| Large · EXTERNAL | River | 5 | 0 | 0 | 703.7 | 0.0 | -1003.0, -1085.2, -1008.8, -1004.4, -973.1 |
| Large · EXTERNAL | Apalis | 0 | 5 | 0 | 703.7 | 0.0 | +495.5, +300.4, +502.9, +458.3, +485.5 |
| Dense | Oban | 0 | 0 | 5 | 561.4 | 659.3 | +1262.0, +1220.7, +1257.2, +1307.8, +1202.6 |
| Dense | River | 0 | 0 | 5 | 561.4 | 1087.0 | +1736.2, +1583.4, +1529.0, +1640.8, +1842.9 |
| Dense | Apalis | 0 | 0 | 5 | 561.4 | 1367.9 | +1959.7, +1909.6, +1923.5, +1955.0, +1975.4 |
| Large · EXTERNAL and 16 MiB work_mem | Oban | 5 | 0 | 0 | 769.0 | 0.0 | -385.3, -413.8, -402.3, -358.0, -420.0 |
| Large · EXTERNAL and 16 MiB work_mem | River | 5 | 0 | 0 | 769.0 | 0.0 | -935.5, -949.6, -1033.0, -974.3, -1176.8 |
| Large · EXTERNAL and 16 MiB work_mem | Apalis | 0 | 5 | 0 | 769.0 | 0.0 | +512.0, +507.4, +483.9, +524.3, +470.9 |
| Short · grouped settlements | Oban | 5 | 0 | 0 | 69.4 | 0.0 | -186.8, -168.4, -157.3, -133.0, -159.0 |
| Short · grouped settlements | River | 0 | 3 | 2 | 69.4 | 0.0 | +63.7, +77.8, +36.6, +18.5, +139.9 |
| Short · grouped settlements | Apalis | 0 | 0 | 5 | 69.4 | 40.7 | +107.1, +110.1, +166.7, +108.4, +179.8 |
| Slow · batch enqueue | Oban | 4 | 1 | 0 | 521.0 | 0.0 | +2.4, -156.6, -144.7, -123.2, -34.5 |
| Slow · batch enqueue | River | 1 | 4 | 0 | 521.0 | 0.0 | +202.1, +159.1, +184.7, -73.9, +202.7 |
| Slow · batch enqueue | Apalis | 0 | 3 | 2 | 521.0 | 0.0 | +187.2, +560.7, +533.4, +417.1, +536.0 |
| Short · batch enqueue and grouped settlements | Oban | 5 | 0 | 0 | 35.6 | 0.0 | -180.1, -208.6, -184.9, -198.2, -270.9 |
| Short · batch enqueue and grouped settlements | River | 2 | 2 | 1 | 35.6 | 0.0 | +29.5, +37.6, -9.9, +35.6, -15.2 |
| Short · batch enqueue and grouped settlements | Apalis | 0 | 2 | 3 | 35.6 | 7.4 | +3.1, +61.7, +30.2, +44.8, +69.1 |
| Short · batch enqueue, grouped settlements and hot checkout | Oban | 5 | 0 | 0 | 36.2 | 0.0 | -195.7, -212.5, -187.5, -297.5, -222.9 |
| Short · batch enqueue, grouped settlements and hot checkout | River | 5 | 0 | 0 | 36.2 | 0.0 | -68.6, -65.9, -4.7, -77.4, -54.2 |
| Short · batch enqueue, grouped settlements and hot checkout | Apalis | 0 | 0 | 5 | 36.2 | 23.2 | +60.0, +58.0, +63.1, +53.1, +74.8 |
| Short · batch enqueue, hot checkout and confirmed teardown | Oban | 5 | 0 | 0 | 37.1 | 0.0 | -226.4, -217.0, -214.7, -245.5, -244.6 |
| Short · batch enqueue, hot checkout and confirmed teardown | River | 3 | 0 | 2 | 37.1 | 0.0 | +50.4, -21.8, -71.3, -25.7, +84.8 |
| Short · batch enqueue, hot checkout and confirmed teardown | Apalis | 0 | 0 | 5 | 37.1 | 12.7 | +52.5, +58.7, +39.0, +44.6, +106.5 |
| Slow · two 32-slot runners per engine and 71 sessions | Oban | 8 | 0 | 0 | 566.1 | 0.0 | -126.8, -43.8, -127.8, -60.2, -121.9, -119.7, -245.3, -135.1 |
| Slow · two 32-slot runners per engine and 71 sessions | River | 6 | 2 | 0 | 566.1 | 0.0 | -67.0, +107.3, -137.5, -256.4, -221.0, -267.8, +42.1, -5.3 |
| Slow · two 32-slot runners per engine and 71 sessions | Apalis | 0 | 8 | 0 | 566.1 | 0.0 | +548.4, +423.1, +324.0, +351.6, +337.8, +281.9, +338.0, +335.2 |

Trusted Mesh runs handlers with application credentials in the shared process. Large rows labeled EXTERNAL apply the same payload storage policy to every engine. The grouped-settlement variant can commit up to eight successful terminal transitions together. [Matched runs and executable identities](scores.json) · [Interactive results](chart.html)
