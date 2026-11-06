fn check(value :: Bool) -> Result<(),CryptoError> do
  if value do Ok(nil) else Err(InternalFailure) end
end
fn repeat(value :: Int, count :: Int) -> Bytes!CryptoError do
  case Bytes.repeat(value, count) do Ok(v) -> Ok(v) Err(_) -> Err(InternalFailure) end
end
fn invalid_key(h :: MlKemHeader64,v :: MlKemPublicVector1152) -> Bool do
  case Crypto.mlkem_validate_public_parts(h,v) do
    Err(InvalidPublicKey) -> true
    _ -> false
  end
end
fn run() -> Result<(),CryptoError> do
  let one = Crypto.mlkem_generate()?
  let (h,v) = Crypto.mlkem_public_parts(one.private_key)?
  check(case Crypto.mlkem_validate_public_parts(MlKemHeader64 {bytes:repeat(0,13)?},MlKemPublicVector1152 {bytes:Bytes.empty()}) do
    Err(InvalidLength(expected, actual)) -> expected == 64 && actual == 13
    _ -> false
  end)?
  let two = Crypto.mlkem_generate()?
  let (other_h,other_v) = Crypto.mlkem_public_parts(two.private_key)?
  Crypto.mlkem_validate_public_parts(h,v)?
  crypto_mlkem_validate_public_parts(h,v)?
  let operation = Crypto.mlkem_validate_public_parts
  operation(h,v)?
  check(invalid_key(h,other_v) && invalid_key(other_h,v))?
  for n in [0,63,65] do
    check(case Crypto.mlkem_validate_public_parts(MlKemHeader64 {bytes:repeat(0,n)?},v) do
      Err(InvalidLength(_, _)) -> true
      _ -> false
    end)?
  end
  for n in [0,1151,1153] do
    check(case Crypto.mlkem_validate_public_parts(h,MlKemPublicVector1152 {bytes:repeat(0,n)?}) do
      Err(InvalidLength(_, _)) -> true
      _ -> false
    end)?
  end
  let noncanonical_header = case Bytes.from_hex("a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6a6364630f3002f1d94500f3ed876352ea0ec019a697fe4d0a07d8ff71750dbc991") do
    Ok(value) -> value
    Err(_) -> return Err(InternalFailure)
  end
  check(invalid_key(MlKemHeader64 {bytes:noncanonical_header},MlKemPublicVector1152 {bytes:repeat(255,1152)?}))?
  # Validation does not consume wrappers or create private resources. The same
  # key still performs the established full/incremental operation afterward.
  let (state,first,shared) = Crypto.mlkem_encapsulate1(h)?
  let second = Crypto.mlkem_encapsulate2(state,v)?
  let opened = Crypto.mlkem_decapsulate_parts(one.private_key,first,second)?
  check(Bytes.secure_equals(Crypto.hmac_sha256_tag(shared,Bytes.empty())?,Crypto.hmac_sha256_tag(opened,Bytes.empty())?))?
  Secret.destroy(state)
  Secret.destroy(shared)
  Secret.destroy(opened)
  Secret.destroy(one.private_key)
  Secret.destroy(two.private_key)
  Ok(nil)
end
fn main() do
  case run() do
    Ok(_) -> println("C1_PUBLIC_VALIDATION_EXECUTED")
    Err(_) -> panic("public validation fixture refused")
  end
end
