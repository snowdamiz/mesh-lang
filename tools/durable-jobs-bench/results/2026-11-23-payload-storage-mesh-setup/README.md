# Native Mesh payload storage SETUP

Both original Mesh brokers completed the 64 × 512 KiB large workload on the same GNU/Linux AMD64 worker. Each run recorded 64 one-attempt completions and 128 independently committed, ordered application writes. A post-settlement audit matched all 64 payload hashes to the native ledger. Each run stored 33,559,808 bytes of JSONB arguments, with all 64 arguments uncompressed.

Both installations set only the storage strategy of the `args` column after migrating an empty disposable database and before submission. The DDL and its readbacks fault independently of the benchmark window. The setting affects writes to all rows of that table. Existing production rows may retain their previous storage until rewritten. The separate repeated-character fixture quantifies the cost for compressible inputs. [Local component](../2026-11-23-large-storage-component/README.md)

The stock producer and direct-file producer retained their frozen executable pins and their original transport. Both used the same protected worker, claim protocol, application sessions and durability settings as the current Mesh baseline. Native timing belongs to the separate measured series. [Classic broker smoke](mesh-external/reconciled.json) · [Direct-file broker smoke](mesh-direct-external/reconciled.json) · [Measured series inventory](large.json)

To replay the raw SETUP receipts from the checkout root, run `python3 -B tools/durable-jobs-bench/native/payload-storage/verify.py`. To stage new smokes, pass a fresh output directory to the sibling `controller.py` during a native SETUP reservation. The archived installer digest, initial adapter pins and score inventory hashes identify the exact inputs. [Completed stage](stage-complete.json)
