from Solana.Read import Pubkey

pub struct ProgramDerivedAddress do
  address :: Pubkey
  bump :: Int
end

fn seed_bytes(program :: Pubkey, seeds :: List<Bytes>, maximum :: Int) -> Bytes!String do
  if Bytes.length(program.bytes) != 32 || List.length(seeds) > maximum do
    return Err("SOLANA_PDA: invalid program or seed count")
  end
  for seed in seeds do
    if Bytes.length(seed) > 32 do
      return Err("SOLANA_PDA: seed exceeds 32 bytes")
    end
  end
  join(seeds, Bytes.empty())
end

fn join(parts :: List<Bytes>, output :: Bytes) -> Bytes!String do
  if List.length(parts) == 0 do
    Ok(output)
  else
    join(List.tail(parts), Bytes.concat(output, List.head(parts))?)
  end
end

fn derive(program :: Pubkey, seeds :: Bytes) -> Option<Pubkey>!String do
  let suffix = Bytes.concat(program.bytes, Bytes.from_utf8("ProgramDerivedAddress"))?
  let digest = Crypto.sha256(Bytes.concat(seeds, suffix)?)
  let on_curve = case Crypto.ed25519_is_on_curve(digest) do
    Err(_) -> return Err("SOLANA_PDA: curve membership check failed")
    Ok(value) -> value
  end
  if on_curve do
    Ok(None)
  else
    Ok(Some(Pubkey { bytes: digest }))
  end
end

## Includes all supplied seeds verbatim; callers append their chosen bump.
## Solana concatenates seeds without lengths, so callers must choose an
## unambiguous schema (fixed-width fields or explicit framing).
pub fn create_program_address(program :: Pubkey, seeds :: List<Bytes>) -> Pubkey!String do
  case derive(program, seed_bytes(program, seeds, 16)?)? do
    None -> Err("SOLANA_PDA: seeds produce an on-curve address")
    Some(address) -> Ok(address)
  end
end

## Finds the canonical highest bump, matching solana-address 2.7's search.
## At most 15 caller seeds leave one of the 16 seed slots for that bump.
pub fn find_program_address(program :: Pubkey,
  seeds :: List<Bytes>) -> ProgramDerivedAddress!String do
  search(program, seed_bytes(program, seeds, 15)?, 255)
end

fn search(program :: Pubkey, seeds :: Bytes, bump :: Int) -> ProgramDerivedAddress!String do
  if bump == 0 do
    return Err("SOLANA_PDA: no viable bump")
  end
  let byte = case Bytes.from_list([bump]) do
    Err(_) -> return Err("SOLANA_PDA: invalid bump")
    Ok(value) -> value
  end
  case derive(program, Bytes.concat(seeds, byte)?)? do
    None -> search(program, seeds, bump - 1)
    Some(address) -> Ok(ProgramDerivedAddress { address: address, bump: bump })
  end
end
