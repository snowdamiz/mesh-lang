# Mesh Lightning

`Lightning.Invoice.parse_invoice(String)` verifies a bounded BOLT 11 invoice and
returns typed public payment fields. It performs no networking, holds no wallet,
and never sends a payment. The package uses the public runtime primitive
`Lightning.invoice_details(String) -> Bytes!String`.

Verification includes Bech32 checksum/case, secp256k1 signature and payee recovery,
mandatory field cardinality, unknown required feature bits and whole
millisatoshis. A strict check rejects malformed fixed-width `p`, `s`, `h` and `n`
fields that the dependency otherwise preserves as unknown fields. Unknown tags
and optional unknown features retain BOLT behavior. All 31 upstream vectors at
the revision in `tests/vectors/lightning/source.json` run through the runtime and
compiled Mesh at optimization levels 0 and 2.

Inputs are bare invoices, not `lightning:` URIs, and are limited to 7,089 ASCII
bytes before parsing. All parser failures return `invalid Lightning invoice`
without reflecting input. Uppercase invoices are accepted. Amountless invoices
parse but `fixed_invoice_amount` rejects amountless or zero-value requests.
Callers must enforce their expected network, amount, payee, hash, description,
expiry, fallback policy and invoice purpose before authorizing a payment.

Networks are `0` Bitcoin mainnet, `1` testnet, `2` regtest, `3` signet and `4`
simnet. All amounts are exact `U64` millisatoshis. Time is Unix seconds.
`invoice_expires_at` checks addition overflow. An invoice signature authenticates
its declared payee; it does not prove that payee is the intended merchant.

The internal 127-byte result is versioned: `01 4c 4e 49`, network byte, flags
byte (bit 0: amount present, bit 1: fallback present), amount/creation/expiry as
three big-endian u64 values, payment hash (32 bytes), compressed payee (33 bytes),
and SHA256 description commitment (32 bytes). A direct description is hashed;
a hashed description retains its declared digest. No invoice text is rendered
as HTML and no payment-secret field is exported by this compact view.

The runtime pins `lightning-invoice = 0.34.1` without default features, using its
Bitcoin/secp256k1 verifier. This adds public-signature verification only; Mesh's
private-key operations continue to use their existing providers. Mobile target
builds, dependency review, fuzzing and release gates remain required before a
release claim.

## Hash commitments for hold invoices

`HashPreimage.generate() -> HashPreimage!CryptoError` creates exactly 32 random
bytes in Mesh's affine secret table. `HashPreimage.commitment(borrow value)`
returns SHA256 of those bytes. `HashPreimage.reveal(value)` consumes the handle
and **intentionally publishes** the preimage as ordinary `Bytes`.

There is no import, conversion from `SecretBytes`, or caller-selected material.
This API cannot reveal signing keys or message secrets. The resource has the
usual owner, generation, quota, destruction, actor-cleanup and serialization
restrictions. Errors do not contain material. Storage uses
`HashPreimage.seal_for_storage` / `unseal_from_storage` with resource kind 11,
context purpose 20, a zero session ID and exactly 32 bytes of material.

The application must durably authorize settlement before revealing. A restored
sealed copy can reveal again so a lost network response can be recovered;
encrypted storage is not an application replay fence. Revealed values are public
protocol data and no longer have secret-memory erasure guarantees.

Primary references: [BOLT 11](https://github.com/lightning/bolts/blob/191462825ae341f0a27098765c075f01ca9bc6c2/11-payment-encoding.md),
[LND invoice API](https://github.com/lightningnetwork/lnd/blob/master/lnrpc/invoicesrpc/invoices.proto),
and [lightning-invoice API](https://docs.rs/lightning-invoice/0.34.1/lightning_invoice/).
