# mesh-anchor

Pure Mesh validation for Anchor accounts.

`discriminator` calculates the first eight bytes of
`sha256("account:<AccountName>")`. `account_payload` requires an explicit
32-byte expected program owner and validates it before the discriminator.

For versioned layouts, pass an `AccountLayout` to `versioned_payload`. The
returned bytes exclude the discriminator but retain the version byte. An
IDL-generated or handwritten field mapper can pass that payload to
`mesh-borsh`; keeping field mapping outside this package makes the program ID,
account name, version, and field order explicit at the call site.
