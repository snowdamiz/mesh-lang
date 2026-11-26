# Older jobs settled before the next SLOW handlers entered

In the five canonical one-runner SLOW blocks, 330–402 distinct older jobs settled between the grant timestamp for a different batch and the first recorded handler entry in that batch. The dispatcher ordinarily checks the split claim lane after it finishes polling existing attempts. The available ledgers time the database grant and the handler entry. An untimed held-reply test and a separate native diagnostic can determine whether a checked grant was already available to the dispatcher during those scalar settlements.

| Block | Positive grants | Grants spanning at least two older settlements | Distinct older jobs observed in these grant-to-entry windows, including singletons | Most older settlements in one window |
| --- | ---: | ---: | ---: | ---: |
| 1 | 67 | 30 | 330 | 40 |
| 2 | 55 | 22 | 402 | 46 |
| 3 | 56 | 24 | 391 | 60 |
| 4 | 61 | 28 | 377 | 53 |
| 5 | 55 | 26 | 346 | 44 |

In block 3, the 60 commits spanned 61.2 ms. The first handler entered 62.6 ms after the latest time consistent with the grant's millisecond timestamp. A terminal XID enters the distinct-job column once even when time windows overlap.

The replay verifies the raw hashes for 2,560 jobs, independent application writes, the required hold, and scalar terminal XIDs. Each positive checked call assigns one common timestamp to its grants. Every call without grants returns at most one marker. The number of distinct grant timestamps equals the upper bound on positive calls in every block, so no two positive calls shared a timestamp. Database commit stamps precede the runner's receipt. These intervals therefore price a scheduling opportunity without assigning the elapsed time to the dispatcher.

Run `python3 -B intervening_settlements.py | cmp - intervening-settlements.json` in this directory.
