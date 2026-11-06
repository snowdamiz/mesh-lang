//! Bounded verification of public BOLT 11 invoices. No wallet or payment authority.

use crate::bytes::mesh_bytes_new;
use crate::io::{alloc_result, err_result, MeshResult};
use crate::string::MeshString;
use lightning_invoice::{
    Bolt11Invoice, Bolt11InvoiceDescriptionRef, Currency, RawTaggedField, SignedRawBolt11Invoice,
};
use sha2::{Digest, Sha256};

const INVALID: &str = "invalid Lightning invoice";
const MAX_INVOICE_BYTES: usize = 7089;

// LNI v1: magic4, network1, flags1, amount/created/expiry u64 BE,
// payment hash32, compressed recovered payee33, description SHA25632.
// Flags: bit0 amount present, bit1 fallback present. No payment is performed.
fn invoice_details(input: &str) -> Result<[u8; 127], &'static str> {
    if input.is_empty()
        || input.len() > MAX_INVOICE_BYTES
        || !input.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(INVALID);
    }
    // Bolt11Invoice::from_str checks checksum, signature (including recovery),
    // required feature bits, mandatory field counts and integral millisatoshis.
    let raw = input
        .parse::<SignedRawBolt11Invoice>()
        .map_err(|_| INVALID)?;
    // The library preserves malformed fixed-width fields as unknown fields.
    // BOLT 11 requires readers to reject these even if a valid duplicate exists.
    for field in &raw.raw_invoice().data.tagged_fields {
        if let RawTaggedField::UnknownSemantics(data) = field {
            if data
                .first()
                .is_some_and(|tag| matches!(tag.to_u8(), 1 | 16 | 19 | 23))
            {
                return Err(INVALID);
            }
        }
    }
    let invoice = Bolt11Invoice::from_signed(raw).map_err(|_| INVALID)?;
    let network = match invoice.currency() {
        Currency::Bitcoin => 0,
        Currency::BitcoinTestnet => 1,
        Currency::Regtest => 2,
        Currency::Signet => 3,
        Currency::Simnet => 4,
    };
    let created = invoice.duration_since_epoch().as_secs();
    let expiry = invoice.expiry_time().as_secs();
    created.checked_add(expiry).ok_or(INVALID)?;
    let description: [u8; 32] = match invoice.description() {
        Bolt11InvoiceDescriptionRef::Direct(value) => {
            Sha256::digest(value.to_string().as_bytes()).into()
        }
        Bolt11InvoiceDescriptionRef::Hash(value) => *value.0.as_ref(),
    };
    let mut output = [0u8; 127];
    output[..4].copy_from_slice(b"\x01LNI");
    output[4] = network;
    output[5] = u8::from(invoice.amount_milli_satoshis().is_some())
        | (u8::from(!invoice.fallbacks().is_empty()) << 1);
    output[6..14].copy_from_slice(&invoice.amount_milli_satoshis().unwrap_or(0).to_be_bytes());
    output[14..22].copy_from_slice(&created.to_be_bytes());
    output[22..30].copy_from_slice(&expiry.to_be_bytes());
    output[30..62].copy_from_slice(invoice.payment_hash().as_ref());
    output[62..95].copy_from_slice(&invoice.get_payee_pub_key().serialize());
    output[95..127].copy_from_slice(&description);
    Ok(output)
}

#[no_mangle]
pub extern "C" fn mesh_lightning_invoice_details(input: *const MeshString) -> *mut MeshResult {
    if input.is_null() {
        return err_result(INVALID);
    }
    let input = unsafe { &*input };
    if input.len as usize > MAX_INVOICE_BYTES {
        return err_result(INVALID);
    }
    match invoice_details(unsafe { input.as_str() }) {
        Ok(value) => alloc_result(0, mesh_bytes_new(value.as_ptr(), value.len() as u64).cast()),
        Err(error) => err_result(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors() -> Vec<serde_json::Value> {
        serde_json::from_str(include_str!(
            "../../../tests/vectors/lightning/invoice-test.json"
        ))
        .unwrap()
    }

    #[test]
    fn official_bolt11_vectors_are_verified_before_details_are_exposed() {
        for value in vectors() {
            let result = invoice_details(value["invoice"].as_str().unwrap());
            assert_eq!(
                result.is_ok(),
                value["valid"].as_bool().unwrap(),
                "{}: {result:?}",
                value["description"]
            );
        }
    }

    #[test]
    fn fixed_details_preserve_integer_amount_hash_payee_and_expiry() {
        let cases = vectors();
        let donation = invoice_details(cases[0]["invoice"].as_str().unwrap()).unwrap();
        assert_eq!(&donation[..6], b"\x01LNI\0\0");
        assert_eq!(&donation[6..14], &0u64.to_be_bytes());
        let coffee = invoice_details(cases[1]["invoice"].as_str().unwrap()).unwrap();
        assert_eq!(&coffee[..6], b"\x01LNI\0\x01");
        assert_eq!(&coffee[6..14], &250_000_000u64.to_be_bytes());
        assert_eq!(&coffee[14..22], &1_496_314_658u64.to_be_bytes());
        assert_eq!(&coffee[22..30], &60u64.to_be_bytes());
        assert_eq!(
            coffee[30..62],
            [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 1, 2, 3, 4, 5, 6, 7,
                8, 9, 1, 2
            ]
        );
        assert_eq!(
            coffee[62..95],
            [
                3, 231, 21, 106, 227, 59, 10, 32, 141, 7, 68, 25, 145, 99, 23, 126, 144, 158, 128,
                23, 110, 85, 217, 122, 47, 34, 30, 222, 15, 147, 77, 217, 173
            ]
        );
        assert_eq!(
            invoice_details(&cases[1]["invoice"].as_str().unwrap().to_uppercase()).unwrap(),
            coffee
        );
    }

    #[test]
    fn bounded_plain_invoice_input_has_no_uri_whitespace_or_error_echo() {
        let source = vectors()[1]["invoice"].as_str().unwrap().to_owned();
        for value in [
            String::new(),
            "x".repeat(7090),
            format!("lightning:{source}"),
            format!(" {source}"),
            format!("{source}\n"),
            "private-sentinel".to_owned(),
        ] {
            assert_eq!(invoice_details(&value), Err("invalid Lightning invoice"));
        }
    }
}
