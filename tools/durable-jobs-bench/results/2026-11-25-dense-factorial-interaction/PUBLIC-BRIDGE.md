# Where the public dense runner lost time in the reverse block

By the 768th of 1,024 second application commits, the public grouped runner had accumulated **202.006 ms** of its **208.309 ms** drain deficit to the retained private runner. The first 128 writes accounted for 65.110 ms. After the final application commit, the public runner spent only 2.427 ms longer reaching terminal settlement. The forward rotation's difference was 5.233 ms over the entire drain.

| Rotation and phase | Public drain, ms | Private drain, ms | Public device busy during drain, ms | Private device busy during drain, ms |
|---|---:|---:|---:|---:|
| Forward, measured | 837.563 | 832.330 | 93–180 | 116–218 |
| Reverse, measured | 1,033.764 | 825.455 | 384–523 | 100–200 |
| Forward, warmup | 828.008 | 820.483 | 142–241 | 108–212 |
| Reverse, warmup | 852.739 | 1,290.925 | 194–224 | 752–813 |

Higher device activity accompanied longer drains on **both** runner images. In the reverse measurement the public arm's device flush time was 176–239 ms versus 56–93 ms for the private arm. In the preceding reverse-block warmups, the private arm accumulated 388–419 ms versus the public arm's 85–99 ms. The bounds use successive samples from the PostgreSQL device, major/minor 8:0. Samples strictly inside the drain give the lower bound and the surrounding samples give the upper bound. These counters include all device users. The archives cannot assign a flush to a particular job or establish whether increased device work caused the slower progress.

The public and private arms made 168 and 169 claim calls in the forward measurement, and 194 and 167 in reverse. Terminal transactions were 163 and 164 forward, and 188 and 162 in reverse. The reverse public arm used 579.296 runner CPU-ms against its 554.181 ceiling. The observed workload differences complicate attributing that excess to the writer implementation. The original pilot qualified in one of two rotations. The warmups provide diagnostic evidence only.

From this directory, run `python3 -B public_bridge.py | cmp - public-bridge.json`. The replay reads the owner's committed inputs at `f0ae2105488f4630d77406c9fd5f739597d023ad`, verifies all 16 raw hashes and preserves the original paired decisions. Each 128-write window ends at the corresponding ordered durable effect, independent of job ID. [Window and device measurements](public-bridge.json)
