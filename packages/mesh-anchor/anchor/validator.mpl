pub struct AccountLayout do
  account_name :: String
  version_offset :: Int
  version :: Int
  minimum_payload_bytes :: Int
end

pub fn discriminator(account_name :: String) -> Bytes ! String do
  (("account:" <> account_name)
    |> Crypto.sha256()
    |> Bytes.from_hex()) ?
    |> Bytes.slice(0, 8)
end

fn validate_owner(actual :: Bytes, expected :: Bytes) -> Int ! String do
  if Bytes.length(actual) != 32 do
    Err("ANCHOR_OWNER: actual owner must be 32 bytes")
  else
    if Bytes.length(expected) != 32 do
      Err("ANCHOR_OWNER: expected owner must be 32 bytes")
    else
      if actual
        |> Bytes.secure_equals(expected) do
        Ok(0)
      else
        Err("ANCHOR_OWNER: account owner mismatch")
      end
    end
  end
end

pub fn account_payload(data :: Bytes,
actual_owner :: Bytes,
expected_owner :: Bytes,
account_name :: String) -> Bytes ! String do
  (validate_owner(actual_owner, expected_owner)) ?
  if Bytes.length(data) < 8 do
    Err("ANCHOR_DISCRIMINATOR: account data is shorter than 8 bytes")
  else
    let expected = (account_name
      |> discriminator()) ?
    if (data
      |> Bytes.slice(0, 8)) ?
      |> Bytes.secure_equals(expected) do
      data
        |> Bytes.slice(8, Bytes.length(data) - 8)
    else
      Err("ANCHOR_DISCRIMINATOR: account discriminator mismatch")
    end
  end
end

pub fn versioned_payload(data :: Bytes,
actual_owner :: Bytes,
expected_owner :: Bytes,
layout :: AccountLayout) -> Bytes ! String do
  if layout.minimum_payload_bytes < 0 do
    Err("ANCHOR_LAYOUT: minimum payload size must be non-negative")
  else
    if layout.version_offset < 0 do
      Err("ANCHOR_LAYOUT: version offset must be non-negative")
    else
      if layout.version < 0 || layout.version > 255 do
        Err("ANCHOR_LAYOUT: version must fit one byte")
      else
        let payload = (data
          |> account_payload(actual_owner, expected_owner, layout.account_name)) ?
        if Bytes.length(payload) < layout.minimum_payload_bytes do
          Err("ANCHOR_LAYOUT: payload is shorter than the versioned layout minimum")
        else
          if layout.version_offset >= Bytes.length(payload) do
            Err("ANCHOR_LAYOUT: version offset is outside the payload")
          else
            let version = (payload
              |> Bytes.get(layout.version_offset)) ?
            if version == layout.version do
              Ok(payload)
            else
              Err("ANCHOR_VERSION: expected #{layout.version} at payload offset #{layout.version_offset}, got #{version}")
            end
          end
        end
      end
    end
  end
end
