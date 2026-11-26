"""Lower bounds on when fully prepared LARGE activations reached PostgreSQL."""


def arrival_bounds(execution_ms, turns):
    """Turns carry first/last floor-ms job stamps and a client-side receipt upper bound."""
    execution_us = int(execution_ms * 1000)
    previous_last = 0
    after_own_first = 0
    without_early_arrivals = 0
    for first, last, latest_receipt in turns:
        assert previous_last <= first <= last <= latest_receipt
        after_own_first += latest_receipt - first
        without_early_arrivals += latest_receipt - previous_last
        previous_last = last
    early = max(0, execution_us - without_early_arrivals)
    return dict(backend_execution_us_floor=execution_us,
                aggregate_before_own_first_stamp_us_min=max(0, execution_us - after_own_first),
                max_execution_if_no_overlap_with_preceding_turn_us=without_early_arrivals,
                aggregate_lead_before_preceding_last_stamp_us_min=early,
                longest_single_lead_us_min=early // (len(turns) - 1))
