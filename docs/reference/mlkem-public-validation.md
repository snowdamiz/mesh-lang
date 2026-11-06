# Validate public ML-KEM-768 parts

`Crypto.mlkem_validate_public_parts(header, vector)` accepts `MlKemHeader64` and
`MlKemPublicVector1152` and returns `Result<(), CryptoError>`. It checks exact
64/1152-byte widths, then the existing libcrux binding and canonical coefficient
validation. Correctly sized invalid keys return `InvalidPublicKey`; bad widths
return `InvalidLength`. The flat spelling is `crypto_mlkem_validate_public_parts`.

This operation uses only public input. It requests no entropy, needs no actor,
creates no private resource and performs no encapsulation. Success is `Ok(nil)`;
callers must handle the Result. It does not certify peer identity, authorize a
protocol transition, validate an encrypted envelope or declassify labeled input
or caller control. Existing Encaps2 repeats the same validation before computation.
