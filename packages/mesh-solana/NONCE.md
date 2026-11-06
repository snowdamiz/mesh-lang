# Current Solana durable nonce primitives

`Solana.Nonce` decodes current initialized System Program nonce accounts and
constructs the required first instruction in an unsigned legacy message. It
contains no signing, custody or application payment policy.

- `nonce_account(AccountInfo) -> NonceAccountState!String` requires a funded,
  non-executable System-owned account of exactly 80 bytes. Both little-endian
  u32 tags must be 1 (Current, Initialized). It preserves authority, nonce hash
  and the full u64 lamports-per-signature value. Legacy, unknown, uninitialized,
  truncated and extended layouts fail.
- `advance_nonce_account_instruction(Pubkey, Pubkey) -> Instruction!String`
  emits System tag4 with writable nonce account, readonly RecentBlockhashes
  sysvar and readonly signer authority. Every key must be 32 bytes.
- `compile_durable_nonce_message(Pubkey, Pubkey, NonceAccountState,
  List<Instruction>) -> LegacyMessage!String` uses the stored nonce hash and
  prepends the advance before every supplied instruction, including compute
  budget instructions. An additional System nonce-advance instruction fails.
  An empty body makes an advance-only message.

Authenticate the requested account address, finalized context, expected
nonce authority and provider policy before constructing a message. These cannot
be inferred from the account data alone. Existing `Solana.Tx` serialization and
the caller's custody signer produce the complete packet. Reserve one nonce
account/value per in-flight intent and durably persist signed bytes before send.

A consumed nonce is not a transfer receipt: execution failure can consume it
while leaving the intended transfer unapplied. Reconcile the exact signature,
full signed message and finalized execution before declaring success or
releasing an intent. Missing/pruned history must not authorize a second transfer.
Simulations must retain this nonce hash rather than replace it.

This supports the current format. The official documentation warns durable
nonces may be deprecated in a future release; no future deployment compatibility
is implied. See the [current protocol contract](https://solana.com/docs/core/transactions/durable-nonces),
[SDK Versions](https://github.com/anza-xyz/solana-sdk/blob/5f79ee8637285d7ec2d39669142cafb7854530e1/nonce/src/versions.rs),
[SDK state](https://github.com/anza-xyz/solana-sdk/blob/5f79ee8637285d7ec2d39669142cafb7854530e1/nonce/src/state.rs),
and [SDK System instruction](https://github.com/anza-xyz/solana-sdk/blob/ab3e8a14bf2bab443bb8af32e95efe2fb65944db/system-interface/src/instruction.rs).

The Morse local proof compares complete compiled Mesh messages with SDK 3.3.0
(after legal account-order normalization), checks real Ed25519 signatures, and
executes the System Program in an isolated Agave bank. It includes execution
failure and nonce replay. LiteSVM0.17 is insufficient for the failure case: it
withdraws the failed transaction fee but does not commit the advanced nonce.
