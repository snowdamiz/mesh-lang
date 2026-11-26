# Verify the signed dense comparison from a fresh Git checkout

The signed verifier reads three historical Linode release receipts from their original paths under `target/`. Git preserves those receipts in the dense checkout archive. From the root of a fresh checkout or Git export, restore just those three files and run the verifier with Python 3.11 or newer and OpenSSL 3.

```sh
results=tools/durable-jobs-bench/results
packet="$results/2026-11-24-dense-product-d2-transfer"
archive="$results/2026-11-24-dense-two-runner-checkout/evidence/dense-native-portable.tar.gz"
receipt=target/linode-jobs-stepchange/requests/2026-11-24-dense-two-runner-checkout
tar -xzkf "$archive" \
    "$receipt-groups-1-2.released.json" \
    "$receipt-import-retry-groups-1-2.released.json" \
    "$receipt-import-retry-groups-3-6.released.json"
python3 -B "$packet/transfer.py" check
```

Run this in an offline checkout, separate from the controller's working tree. The verifier checks the request signature, six planned orders and 121 historical raw-arm digests. The historical measurements belong to the experimental runner. The public product comparison requires its own Linux build receipt and measurements.
