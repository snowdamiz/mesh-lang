# Bounded grants on the public Jobs schema

The exact function change used in the PG17 native trial also applies to the individual-settlement Jobs schema at `1e70f18d`. In disposable PostgreSQL 17.11, 18.7 and 19.0 databases, the old and replacement functions returned identical results for the nine contract cases and both concurrent row replacement cases. Each candidate returned both jobs in order when the first row’s context changed before locking.

An ordinary EXPLAIN after six empty prepared calls and then a committed backlog showed a namespace-only primary-key scan for stock on all three versions. The replacement had index conditions on both namespace and job ID on all three. These are untimed correctness and plan checks on macOS. The native throughput evidence belongs to the grouped PostgreSQL 17.11 trial.

Re-running the older schema migration restores the older function body. Re-running the candidate schema migration restores the bounded lookup, and repeating it preserves that body. Function OID, ownership, ACL, security mode, volatility and settings matched throughout. A mixed-version rollout would therefore need to account for which binary last ran the migration.

From this directory, pass the matching server’s bin directory to each invocation.

```sh
python3 -B tiers.py /opt/homebrew/opt/postgresql@17/bin 17.11 | cmp - tier-17.json
python3 -B tiers.py ../../../../target/native-qualified-pool-transfer/postgres/18.7/bin 18.7 | cmp - tier-18.json
python3 -B tiers.py ../../../../target/native-qualified-pool-transfer/postgres/19.0/bin 19.0 | cmp - tier-19.json
```
