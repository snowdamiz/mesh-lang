"""Validate lossless session charge accounting and reduce the observer screen."""
from collections import Counter, defaultdict
import json

BUSY = (0, 1, 2, 5, 6)
TICK_NS = 10_000_000


def require(ok, message):
    if not ok:
        raise ValueError(message)


def read_report(content, enabled):
    header, *rows = map(json.loads, content.splitlines())
    sessions = [row for row in rows if "session" in row]
    keys = [row for row in rows if "slot" in row]
    require(header["version"] == 1 and header["enabled"] == int(enabled)
            and header["postmaster"] > 0 and header["parallel_seen"] in (0, 1)
            and 0 < header["registered"] == len(sessions) <= 1024
            and len(sessions) + len(keys) == len(rows), "incomplete postmaster exit report")
    require([s["session"] for s in sessions] == list(range(len(sessions)))
            and all(s["exited"] and not s["clock_failed"] and s["auth"] == s["exit"] == 0
                    and s["pid"] > 0 and s["cpu_ns"] > 0 for s in sessions),
            "missing, failed or unclean backend")
    require(bool(keys) is enabled, "statement reporter mode differs")
    charged = Counter()
    seen = set()
    for row in keys:
        slot = row["slot"]
        key = slot, row["db"], row["role"], row["query"]
        require(0 <= slot < len(sessions) and key not in seen and row["role"] > 0
                and row["db"] > 0 and -(2**63) <= int(row["query"]) < 2**63
                and row["cpu_ns"] >= 0 and len(row["calls"]) == 5
                and all(n >= 0 for n in row["calls"]) and 0 <= row["errors"] <= sum(row["calls"])
                and sum(row["calls"]) > 0, "overflow or malformed/duplicate statement charge")
        seen.add(key)
        charged[slot] += row["cpu_ns"]
    require(all(charged[s["session"]] <= s["cpu_ns"] for s in sessions), "statement CPU exceeds session CPU")
    # getrusage rounds the two component clocks to microseconds per process.
    require(sum(s["cpu_ns"] for s in sessions) <= header["children_cpu_ns"] + len(sessions) * 2000
            and header["postmaster_cpu_ns"] > 0, "backend CPU exceeds reaped child CPU")
    return header, sessions, keys


def host_work(points, start_us, end_us):
    first, last = points[0], points[-1]
    require(first["epoch_us"] < start_us < end_us < last["epoch_us"], "host bracket misses the arm")
    ticks = [last["cpu"][cpu][i] - first["cpu"][cpu][i] for cpu in ("cpu0", "cpu1") for i in BUSY]
    require(min(ticks) >= 0, "host CPU counter regressed")
    return dict(total_duration_ns=last["monotonic_ns"] - first["monotonic_ns"],
                host_busy_ns=sum(ticks) * TICK_NS,
                host_lower_ns=sum(max(0, value - 1) for value in ticks) * TICK_NS,
                host_upper_ns=sum(value + 1 for value in ticks) * TICK_NS)


def attribute(content, dictionary, enabled, engine, roster):
    header, sessions, keys = read_report(content, enabled)
    catalog = {(r["db"], r["role"], r["query"]): r for r in dictionary["rows"]}
    require(len(catalog) == len(dictionary["rows"]) and dictionary["dealloc"] == 0, "statement dictionary lost keys")
    for entry in roster:
        require(any((s["pid"], s["login"], s["database"]) == (entry["backend"], entry["role"], "mesh")
                    for s in sessions), "readiness TLS backend missing at exit census")
    logins = defaultdict(Counter)
    families = Counter()
    top = {}
    stage = Counter()
    for row in keys:
        session = sessions[row["slot"]]
        login = session["login"]
        key = row["db"], row["role"], row["query"]
        match = catalog.get(key)
        family = match["family"] if match else "unmapped"
        # Treat source roles as claims about login authority only. SET ROLE can
        # change the executor key and SECURITY DEFINER can change nested roles.
        logins[login]["hook_cpu_ns"] += row["cpu_ns"]
        logins[login][family] += row["cpu_ns"]
        families[family] += row["cpu_ns"]
        stage[family] += row["calls"][1] + row["calls"][4]
        identity = login, session["database"], *key
        if identity not in top:
            top[identity] = dict(login=login, database=session["database"], db=key[0], role=key[1],
                                 query=key[2], dictionary=match, cpu_ns=0, errors=0, calls=[0] * 5)
        record = top[identity]
        record["cpu_ns"] += row["cpu_ns"]
        record["errors"] += row["errors"]
        record["calls"] = [a + b for a, b in zip(record["calls"], row["calls"], strict=True)]
    if enabled:
        require(stage["settle"] >= (1024 if engine == "mesh" else 1) and stage["grant"] > 0
                and stage["application-write"] >= 2048 and stage["enqueue"] > 0 and stage["observer"] > 0,
                f"expected workload statements missing: {dict(stage)}")
    for session in sessions:
        logins[session["login"]]["session_cpu_ns"] += session["cpu_ns"]
        logins[session["login"]]["sessions"] += 1
    for row in logins.values():
        row["session_residual_ns"] = row["session_cpu_ns"] - row["hook_cpu_ns"]
    session_cpu = sum(s["cpu_ns"] for s in sessions)
    hook_cpu = sum(families.values())
    return dict(session_cpu_ns=session_cpu, hook_cpu_ns=hook_cpu, session_residual_ns=session_cpu - hook_cpu,
                postmaster_cpu_ns=header["postmaster_cpu_ns"],
                other_reaped_cpu_ns=header["children_cpu_ns"] - session_cpu,
                postgres_cpu_sample_ns=header["postmaster_cpu_ns"] + header["children_cpu_ns"],
                postmaster=header["postmaster"], parallel_seen=bool(header["parallel_seen"]),
                sessions=len(sessions), logins=dict(logins), families=dict(families),
                top_level=sorted(top.values(), key=lambda row: -row["cpu_ns"]))


def perturbation(a, on, b):
    off_mid = (a + b) / 2
    spread = abs(a - b)
    offset = on - off_mid
    return dict(off_mid=off_mid, duplicate_spread=spread, on_minus_off_mid=offset,
                passes=off_mid > 0 and spread <= 0.10 * off_mid
                and abs(offset) <= max(0.05 * off_mid, spread / 2))


METRICS = ("total_duration_ns", "host_busy_ns", "submit_ns", "drain_ns")


def interpret(groups, gap):
    screens = {}
    for group, lanes in groups.items():
        screens[group] = {
            engine: {metric: perturbation(*(lanes[f"{engine}-{case}"][metric]
                                            for case in ("off-a", "on", "off-b")))
                     for metric in METRICS} for engine in ("mesh", "river")
        }
    eligible = all(row["passes"] for group in screens.values() for engine in group.values() for row in engine.values())
    changes = {"pure_wait_overlap": {"reduction_ns": 0, "excludes_smallest_gap": True}}
    serial = not any(lanes[f"{engine}-on"]["postgres"]["parallel_seen"]
                     for lanes in groups.values() for engine in ("mesh", "river"))
    if eligible and serial:
        scopes = {"settle_statement": ("parity_subject", "settle"),
                  "grant_statement": ("parity_subject", "grant"),
                  "application_insert_statements": ("compiled_caller", "application-write"),
                  "broker_enqueue_statement": ("parity_producer", "enqueue"),
                  "observer_statements": (None, "observer")}
        for change, (login, family) in scopes.items():
            rows = []
            for lanes in groups.values():
                pg = lanes["mesh-on"]["postgres"]
                roles = [pg["logins"][login]] if login else list(pg["logins"].values())
                observed = sum(role[family] for role in roles)
                upper = observed + sum(role["unmapped"] for role in roles)
                rows.append(dict(observed_ns=observed, scoped_upper_ns=upper))
            low = min(row["observed_ns"] for row in rows)
            high = max(row["scoped_upper_ns"] for row in rows)
            changes[change] = dict(rows=rows, excludes_smallest_gap=high < min(gap), excludes_largest_gap=high < max(gap),
                                   needed_fraction_of_smallest_observation=(max(gap) / low if low else None))
    deltas = {}
    if eligible and serial:
        for group, lanes in groups.items():
            mesh = lanes["mesh-on"]["postgres"]["families"]
            river = lanes["river-on"]["postgres"]["families"]
            deltas[group] = {family: mesh.get(family, 0) - river.get(family, 0)
                             for family in sorted(mesh.keys() | river.keys())}
    return dict(screens=screens, attribution_eligible=eligible and serial, mesh_minus_river_hook_ns=deltas,
                parallel_worker_seen=not serial, historical_required_cpu_ns=gap,
                statement_only_changes=changes)
