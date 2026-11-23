# Dense Jobs CPU samples

The Linode ran two separate diagnostic no-op arms after the trace-off baseline was sealed. Linux `perf` sampled CPU clock events at 199 Hz on CPUs 0 and 1. The sampled window begins at the first durable enqueue commit and ends at the final terminal commit. Each arm reconciled 1,024 successful jobs and 2,048 ordered application commits. The diagnostic runs contribute zero baseline observations.

The Mesh samples attributed 1,019 of 2,202 parsed samples to PostgreSQL and 707 to the one-shot handler process tree. Of the handler samples, 567 were in the kernel, including page faults and page-table cleanup. Oban attributed 275 of 639 samples to PostgreSQL and 299 to the BEAM runner processes that execute handlers. Sampling did not isolate CPU for individual SQL statements. [Classification counts](counts.json), [Mesh symbols](mesh/symbols.txt), [Oban symbols](oban/symbols.txt)

The root driver kept both diagnostic arm archives on the host. The host path and profiling commands are in `host.json` and the engine request records. The saved text exports cover the commit-to-terminal windows. They omit SQL arguments.
