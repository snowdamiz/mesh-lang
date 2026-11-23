# Native comparison request

The controller request is `target/linode-jobs-stepchange/requests/ordinary-trusted-product-db5937e7.json`. It pins the product source at `db5937e7e8988f935e9be16b68a2561c2250cc27`. The archive's SHA-256 is `5520b50b8a16535dd413c1135860f46264f768ab2b9f372a5313567555944c94`. The request SHA-256 is `7db062f8276b3380a401be2c717528e96aef675da0f11ebbc624c7b707abdc54`.

Check the request's local pins from the checkout root.

```sh
request=target/linode-jobs-stepchange/requests/ordinary-trusted-product-db5937e7.json
python3 -B proof/jobs-trusted-shared-process/native/setup.py "$request" --check
```

The setup driver requires a new controller assignment for this request ID. The request reserves 15 minutes. It uses the retained Linux x86-64 compiler, existing PostgreSQL 17.11 toolchain, sealed V1 broker, and the original non-executable fixture files. It links one new executable against the product runtime.

The driver runs four diagnostic arms on CPUs 0 and 1. The short workload runs frozen V1 then product. The dense workload, called `noop` in the broker, runs product then frozen V1. Both arms use the same input digest, producer and handler source, native bindings, broker, database settings, and defaults. A separate compiled product variant runs the fault replay. Qualification records the two executable identities and exports `qualified-pair.json` when all checks pass.

For the first matched transfer, the controller can pass that inventory to the frozen interleave driver with `--group 1` and a new series ID under a separate timing assignment. Historical V1 timings retain the original ELF identity. [Frozen V1 results](../../../tools/durable-jobs-bench/results/2026-11-23-ordinary-trusted-v1/README.md)
