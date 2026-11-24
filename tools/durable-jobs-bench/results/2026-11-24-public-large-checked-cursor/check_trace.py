"""Check the trace reducer with synthetic log records only."""
from datetime import datetime, timezone

from trace import analyze


def record(ms, pid, query, duration=0):
    stamp = datetime.fromtimestamp(ms / 1000, timezone.utc).strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]
    return f"{stamp} UTC [{pid}] 1 parity_subject mesh | LOG:  duration: {duration:.3f} ms  {query}"


def chain(start, pid):
    steps = (("statement: BEGIN ISOLATION LEVEL READ COMMITTED", 0),
             ("execute <unnamed>: SELECT status\n\tFROM mesh_jobs.claim_batch_set_v1($1)", 2),
             ("statement: COMMIT", 0),
             ("execute <unnamed>: WITH clock AS MATERIALIZED (SELECT mesh_jobs.now_ms() AS now_ms)\n"
              "\tSELECT j.scheduled_at_ms > (SELECT now_ms FROM clock)", 0),
             ("statement: BEGIN ISOLATION LEVEL READ COMMITTED", 0),
             ("execute <unnamed>: SELECT * FROM mesh_jobs.claim_batch_v1($1)", 0),
             ("statement: COMMIT", 0))
    return [record(start+i, pid, q, elapsed) for i, (q, elapsed) in enumerate(steps)]


def check():
    logs = chain(1000, 10) + chain(1001, 20)
    logs += [record(2000, 10, "statement: BEGIN ISOLATION LEVEL READ COMMITTED"),
             record(2001, 10, "statement: ROLLBACK")]
    result = analyze("\n".join(logs), 1_000_500, 1_002_999, 1_010_000)
    assert len(result["sequences"]) == 2 and result["cross_enqueue"] == 1
    assert result["before_final_grant_set_coverage_ms"] == 2.499
    assert result["before_terminal_set_coverage_ms"] == 2.5
    print("PASS synthetic interleaved fallback, rollback and overlap accounting")


if __name__ == "__main__":
    check()
