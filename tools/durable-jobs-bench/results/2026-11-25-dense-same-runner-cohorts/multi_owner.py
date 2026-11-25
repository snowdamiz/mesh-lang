"""Price terminal request exchanges when at least three intact owners participate."""
from functools import lru_cache
from itertools import combinations
import json

from cross_runner import EVIDENCE, load_ledger, transactions


def maximum_saved_exchanges(events, cutoff, span_us, lag_us):
    # An unmodified local request needs one exchange; crossed preparation and
    # COMMIT need two. Enumerate subsets including their earliest observed commit.
    ordered = sorted(events, key=lambda terminal: (terminal.stamp, terminal.xid))
    options = [[] for _ in ordered]
    for i, first in enumerate(ordered):
        neighbors = []
        for j in range(i + 1, len(ordered)):
            other = ordered[j]
            if other.stamp - first.stamp > span_us:
                break
            if other.owner != first.owner and other.width + first.width < 8:
                neighbors.append(j)
        for k in range(2, min(len(neighbors), 7) + 1):
            for others in combinations(neighbors, k):
                group = [first, *(ordered[j] for j in others)]
                if len({e.owner for e in group}) != len(group) or sum(e.width for e in group) > 8:
                    continue
                xid = min(e.xid for e in group)
                if max(e.release_after_xid for e in group) >= xid:
                    continue
                if max(e.release_after for e in group) + lag_us > cutoff[xid]:
                    continue
                mask = 1 + sum(1 << (j-i) for j in others)
                options[i].append((mask, len(group)-2))

    # Occupied bits refer to later requests already assigned to an earlier group.
    # Disjoint groups may interleave and can skip any intervening local requests.
    @lru_cache(maxsize=None)
    def best(index, occupied):
        if index == len(ordered):
            return 0
        answer = best(index + 1, occupied >> 1)
        for mask, saving in options[index]:
            if not (mask & occupied):
                answer = max(answer, saving + best(index + 1, (occupied | mask) >> 1))
        return answer

    return best(0, 0)


def main():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["exit_status"] == 0
    seals = json.loads((EVIDENCE / "summary.json").read_bytes())["raw_sha256_by_rotation"]
    results = []
    for rotation in range(1, 6):
        raw, = EVIDENCE.glob(f"blocks/{rotation:02}-*/measured-B/*/raw.tar.gz")
        seal = seals[str(rotation)]["measured-B"]
        ledger, sql = load_ledger(raw, seal, with_queue=True)
        events, cutoff = transactions(ledger["jobs"])
        assert len(events) == sql["terminal_xids"]
        effects = sorted(j["effect_committed_us"] for j in ledger["jobs"])
        middle = [e for e in events if effects[255] <= e.stamp <= effects[767]]
        rows = []
        for span in (250, 1000, 4000):
            for lag in (0, 1000):
                rows.append(dict(span_us=span, lag_us=lag,
                                 whole=maximum_saved_exchanges(events, cutoff, span, lag),
                                 middle=maximum_saved_exchanges(middle, cutoff, span, lag)))
        results.append(dict(rotation=rotation, raw_sha256=seal, terminal_xids=len(events),
                            middle_terminal_xids=len(middle), gross_exchanges_saved=rows))
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
