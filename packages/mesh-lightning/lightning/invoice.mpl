# The public runtime verifies BOLT 11; this package gives its fixed public
# result a typed Mesh interface. It never opens a connection or sends money.
pub struct LightningInvoice do
  network :: Int
  amount_msat :: Option<U64>
  created_at :: U64
  expiry_seconds :: U64
  payment_hash :: Bytes
  payee :: Bytes
  description_hash :: Bytes
  has_fallback :: Bool
end

fn wide(value :: Bytes, offset :: Int) -> U64!String do
  case Bytes.read_u64_be(value, offset) do
    Ok(number)
    Err(_) -> Err("invalid_lightning_details")
  end
end

pub fn parse_invoice(input :: String) -> LightningInvoice!String do
  let value = Lightning.invoice_details(input)?
  if Bytes.length(value) != 127 || Bytes.to_hex(Bytes.slice(value, 0, 4)?) != "014c4e49" do
    return Err("invalid_lightning_details")
  end
  let flags = Bytes.get(value, 5)?
  Ok(LightningInvoice {
    network: Bytes.get(value, 4)?,
    amount_msat: if flags == 1 || flags == 3 do
      Some(wide(value, 6)?)
    else
      None
    end,
    created_at: wide(value, 14)?,
    expiry_seconds: wide(value, 22)?,
    payment_hash: Bytes.slice(value, 30, 32)?,
    payee: Bytes.slice(value, 62, 33)?,
    description_hash: Bytes.slice(value, 95, 32)?,
    has_fallback: flags == 2 || flags == 3
  })
end

pub fn invoice_expires_at(value :: LightningInvoice) -> U64!String do
  U64.add(value.created_at, value.expiry_seconds)
end

# Zero or unspecified amounts need a separate caller-selected donation policy.
pub fn fixed_invoice_amount(value :: LightningInvoice) -> U64!String do
  case value.amount_msat do
    Some(amount) -> do
      if amount == U64.parse("0")? do
        Err("lightning_amount_required")
      else
        Ok(amount)
      end
    end
    None -> Err("lightning_amount_required")
  end
end
