"""Predeclared workload, launch-cost and matched-peer comparisons."""

from statistics import median


CLOCKS = ("submit_ms", "drain_ms")
LAZY = ("lazy-a", "lazy-b")


def gates(scored, phase, lanes):
    errors = []
    rows = {label: scored[f"{phase}-{label}"] for label in lanes}
    for label, row in rows.items():
        if (not 0 <= row["cpu_steal_percent"] <= 1 or not 0 <= row["cpu_iowait_percent"] <= 5
                or not all(field in row and row[field] >= 0
                           for field in ("disk_read_bytes", "disk_write_bytes", "disk_busy_ms"))
                or row["host_samples"] < 2):
            errors.append(f"{phase}/{label}: host noise or disk counters")
        if label in (*LAZY, "warm-river", "retained-river"):
            expected = 67 if label == "warm-river" else 7
            ready = row["ready_physical_clients"]
            valid_ready = 1 <= ready <= expected if label in LAZY else ready == expected
            if not valid_ready or row.get("last_probed_physical_clients", expected) != expected:
                errors.append(f"{phase}/{label}: physical readiness")
    for clock in CLOCKS:
        a, b = (rows[key][clock] for key in LAZY)
        midpoint = (a + b) / 2
        bridge = rows["retained-river"][clock]
        if abs(a - b) > max(75, .05 * midpoint):
            errors.append(f"{phase}/{clock}: rebuilt lazy spread exceeds max(75 ms, 5% of midpoint)")
        if max(abs(bridge - a), abs(bridge - b)) > max(100, .075 * bridge):
            errors.append(f"{phase}/{clock}: bridge difference exceeds max(100 ms, 7.5% of retained River)")
    return errors


def prices(rows, clock):
    mesh, bridge, a, warm, b, oban = (rows[label][clock] for label in
                                     ("mesh", "retained-river", "lazy-a", "warm-river", "lazy-b", "oban"))
    lazy = (a + b) / 2
    perturbation = max(abs(a - b), abs(bridge - a), abs(bridge - b))
    warm_effect = lazy - warm
    warm_threshold = max(.05 * warm, perturbation)
    oban_threshold = max(.05 * oban, perturbation)
    return dict(mesh=mesh, retained_river=bridge, lazy_a=a, lazy_b=b, warm_river=warm, oban=oban,
                lazy_minus_warm=warm_effect, warm_minus_mesh=warm - mesh, oban_minus_mesh=oban - mesh,
                retained_minus_lazy=bridge - lazy, lazy_spread=abs(a - b), warm_threshold=warm_threshold,
                oban_threshold=oban_threshold, mesh_warm_river_throughput_lead_pct=100 * (warm / mesh - 1),
                mesh_oban_throughput_lead_pct=100 * (oban / mesh - 1), warm_river_20pct_margin=warm - 1.2 * mesh,
                oban_20pct_margin=oban - 1.2 * mesh)


def signed_verdict(rows, field, threshold):
    if len(rows) != 6:
        return "confirmation incomplete"
    if all(row[field] > row[threshold] for row in rows):
        return "positive"
    if all(row[field] < -row[threshold] for row in rows):
        return "negative"
    return "inconclusive"


def study(blocks):
    report = {}
    measured = {k: {label.removeprefix("measured-"): row for label, row in block["scores"].items()
                    if label.startswith("measured-")} for k, block in blocks.items()}
    for clock in (*CLOCKS, "adapter_start_to_terminal_ms"):
        rows = [dict(group=k, **prices(row, clock)) for k, row in sorted(measured.items())]
        # The 20% threshold is an official-clock economic screen.
        if clock == "adapter_start_to_terminal_ms":
            for row in rows:
                del row["warm_river_20pct_margin"], row["oban_20pct_margin"]
        report[clock] = dict(blocks=rows, verdicts={
            "lazy_minus_warm": signed_verdict(rows, "lazy_minus_warm", "warm_threshold"),
            "warm_minus_mesh": signed_verdict(rows, "warm_minus_mesh", "warm_threshold"),
            "oban_minus_mesh": signed_verdict(rows, "oban_minus_mesh", "oban_threshold"),
        })
        if clock in CLOCKS:
            report[clock]["mesh_clears_20pct"] = {
                peer: len(rows) == 6 and all(row[peer + "_20pct_margin"] > 0 for row in rows)
                for peer in ("warm_river", "oban")
            }
    startup = {}
    for group, rows in sorted(measured.items()):
        a, b, w = (rows[name] for name in (*LAZY, "warm-river"))
        startup[group] = {
            "lazy_minus_warm_launch_to_settlement_us":
                (a["runner_launch_to_last_settlement_us"] + b["runner_launch_to_last_settlement_us"]) / 2
                - w["runner_launch_to_last_settlement_us"],
            "warm_minus_lazy_launch_to_ready_us":
                w["runner_launch_to_ready_us"] - (a["runner_launch_to_ready_us"] + b["runner_launch_to_ready_us"]) / 2,
            "median_lazy_producer_connection_setup_us":
                median((a["producer_connection_setup"]["elapsed_us"], b["producer_connection_setup"]["elapsed_us"])),
            "warm_producer_connection_setup_us": w["producer_connection_setup"]["elapsed_us"],
            "per_runner_warm_fills": w["pool_fills"],
        }
    report["same_binary_startup"] = startup
    return report
