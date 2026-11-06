# Finalized transaction reads

`Solana.TransactionRead` adds a binary-first, bounded transaction reader and a
fixed `getTransaction` request with `encoding: "base64"`, `commitment:
"finalized"`, and `maxSupportedTransactionVersion: 0`. The query signature is
64 raw bytes; use `Solana.Read.signature(text).bytes` for wallet text input.
`get_finalized_transaction` returns `None` for an unknown transaction and an
error for transport, RPC, execution, framing, signature or metadata failures.
A caller may retry `None` and transport failures without changing the query.
`Solana.Read.rpc_send` binds the returned JSON-RPC ID to the request ID.
RPC HTTP redirects are disabled so a configured provider cannot silently select
a different endpoint, including through a POST-to-GET 303 redirect.

`signed_transaction(wire, queried_signature)` validates canonical ShortU16
framing, full input consumption, the queried first signature, all required
Ed25519 signatures over the exact full message, unique account keys, message
header roles, and every compiled account/program index. It supports legacy and
version-0 messages **without address lookup tables**. Limits are 1,232 wire
bytes, 12 signatures, 64 static keys/instructions/instruction accounts. Other
versions, nonzero lookup counts and trailing bytes fail explicitly. Loaded RPC
addresses must be empty; no unsigned RPC-supplied address can replace a signed
recipient. This deliberately excludes ALT purchases at launch.

`finalized_transaction_from_response(response, signature, expected_id)` also
requires successful execution (`meta.err` must be JSON null), matching wire/RPC
versions, canonical base64, numeric u64 slot/fee/lamport balances, exact account
counts, signed i64-or-null block time, and bounded token balance indices. Token
amounts use the RPC-specified decimal strings and full u64 arithmetic. Quoted
numbers, fractional numbers, out-of-range values, duplicate token balance
indices, and malformed metadata fail. An inner-instruction array is counted;
null recording becomes `-1`, allowing a payment policy to reject uncertainty.
Inner groups and their account/program indices and canonical base58 data are
strictly decoded against the same signed account keys. These RPC-reported CPIs
are metadata; a payment policy must independently restrict their parent and
permitted operations. They are never accepted as an inner-only payment.

The RPC provider remains a trust boundary for finality, execution status,
balance effects and block time. Ed25519 signatures authenticate the message;
they do not prove its inclusion or the provider's chain history. Pin/configure
the provider appropriately. This module is not a Solana light client. Pure
response decoders accept RPC-shaped data for offline replay; do not expose
those decoders as an unauthenticated payment-approval endpoint.

`get_finalized_transaction_outcome` and
`finalized_transaction_outcome_from_response` use the same signature, wire and
metadata checks while preserving execution failure as
`FinalizedTransactionOutcome { transaction, execution_error: Option<String> }`.
The error is canonical JSON: a bounded variant name, or an `InstructionError`
whose in-range instruction index has a variant name or a u32 `Custom` code.
Unknown object variants and malformed errors fail closed. This deliberately
bounded surface supports durable operational reconciliation; a caller must
check `execution_error` before recording a transfer. The original read APIs
continue to reject every execution failure. A changed durable nonce does not
establish success because nonce advancement and fees can survive a failed
transaction.

The signed fixtures in `tests/vectors/solana/finalized-transactions` are
serialized by pinned `@solana/kit 8.4.0`, with signatures independently compared
to Node/OpenSSL Ed25519. Their deterministic keys are public test material and
must never receive funds. `generate.cjs` accepts `SOLANA_KIT_MODULE` pointing to
that installed SDK. Public native tests execute the actual compiled Mesh reader
and loopback HTTP transport, including ID mismatch and second-signature failure.

Protocol references:
[transaction structure](https://solana.com/docs/core/transactions/transaction-structure),
[getTransaction](https://solana.com/docs/rpc/http/gettransaction), and
[RPC JSON metadata](https://solana.com/docs/rpc/json-structures).

# Submission primitives

`Solana.TransactionWrite.serialize_signed_legacy_transaction(message, signatures)`
combines a legacy message and its ordered public signatures into a packet. It
requires exactly the message's signature count and reuses `signed_transaction`
to verify each signature and all framing, roles, indices and size bounds before
returning bytes. It never accepts or holds a secret key. The caller signs the
exact serialized message through Mesh's opaque signing resources, then persists
the resulting packet before submission.

`SignedTransaction.recent_blockhash` exposes the exact authenticated 32-byte
hash from the signed message. `Solana.TransactionSend.send_transaction_request`
authenticates the wire and expected first signature before constructing a
bounded base64 `sendTransaction` request with preflight enabled, confirmed
commitment, `maxRetries: 0`, and the caller's U64 minimum context slot.
`sent_signature_from_response` rejects RPC errors, malformed/noncanonical
signatures and a result differing from the expected signed packet. An RPC
acknowledgment is not confirmation or finality. These functions neither select
an endpoint nor authorize a payment; callers must persist the signed packet and
intent before sending, enforce their expiry policy, and reuse the same packet
through uncertain outcomes. Re-signing a timed-out operation can pay twice.

Reuse `Solana.Read.get_latest_blockhash_request` and
`latest_blockhash_from_response` for blockhash/context/last-valid-height reads.
They preserve complete unsigned 64-bit values without floating-point
conversion. The latter height is a transaction lifetime bound, not evidence
that a previously submitted payment failed or was never included.
See [getLatestBlockhash](https://solana.com/docs/rpc/http/getlatestblockhash) and
[sendTransaction](https://solana.com/docs/rpc/http/sendtransaction).
