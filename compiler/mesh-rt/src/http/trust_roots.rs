//! Request-specific explicit HTTPS trust anchors.

use std::sync::Arc;

use rustls::RootCertStore;
use rustls_pki_types::{pem::PemObject, CertificateDer};
use ureq::tls::Certificate;

pub(super) type TrustRoots = Arc<Vec<Certificate<'static>>>;
const MAX_PEM_BYTES: usize = 65_536;
const MAX_ROOTS: usize = 8;
const BEGIN: &[u8] = b"-----BEGIN CERTIFICATE-----";
const END: &[u8] = b"-----END CERTIFICATE-----";

/// Explicit roots replace webPKI for one request. Only complete certificate
/// blocks with whitespace between them are admitted; no PEM private keys or
/// unrelated text are silently ignored by the general-purpose PEM decoder.
pub(super) fn parse_root_pem(pem: &[u8]) -> Result<TrustRoots, &'static str> {
    if pem.is_empty() || pem.len() > MAX_PEM_BYTES || !pem.is_ascii() {
        return Err("INVALID_TLS_ROOTS: expected 1..65536 PEM bytes");
    }
    let mut remainder = pem.trim_ascii();
    let mut roots = Vec::new();
    let mut parsed = RootCertStore::empty();
    while !remainder.is_empty() {
        if roots.len() == MAX_ROOTS || !remainder.starts_with(BEGIN) {
            return Err("INVALID_TLS_ROOTS: expected at most eight certificate blocks");
        }
        let end = remainder
            .windows(END.len())
            .position(|bytes| bytes == END)
            .ok_or("INVALID_TLS_ROOTS: incomplete certificate block")?
            + END.len();
        let certificate = CertificateDer::from_pem_slice(&remainder[..end])
            .map_err(|_| "INVALID_TLS_ROOTS: invalid certificate PEM")?;
        parsed
            .add(certificate.clone())
            .map_err(|_| "INVALID_TLS_ROOTS: invalid X509 trust anchor")?;
        roots.push(Certificate::from_der(certificate.as_ref()).to_owned());
        remainder = remainder[end..].trim_ascii_start();
    }
    if roots.is_empty() {
        return Err("INVALID_TLS_ROOTS: no certificates");
    }
    Ok(Arc::new(roots))
}

#[cfg(test)]
#[path = "../../tests/support/http_tls.rs"]
pub(super) mod fixture;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_trust_root_parses_only_bounded_certificate_bundles() {
        let fixture = fixture::TlsFixture::new();
        let pem = fixture.pem("ca.pem");
        let parsed = parse_root_pem(&pem).unwrap();
        assert_eq!(parsed.len(), 1);
        for invalid in [
            Vec::new(),
            b"untrusted text".to_vec(),
            b"-----BEGIN CERTIFICATE-----\nYQ==\n-----END CERTIFICATE-----".to_vec(),
            fixture.pem("ca.key"),
            [pem.as_slice(), b"trailing text"].concat(),
            [b"leading text".as_slice(), pem.as_slice()].concat(),
            [pem.as_slice(), fixture.pem("ca.key").as_slice()].concat(),
            vec![b'x'; 65_537],
            pem.repeat(9),
        ] {
            assert!(parse_root_pem(&invalid).is_err());
        }
        assert_eq!(parse_root_pem(&pem.repeat(8)).unwrap().len(), 8);
        let spaced = [b" \r\n".as_slice(), pem.as_slice(), b"\n\t"].concat();
        assert_eq!(parse_root_pem(&spaced).unwrap().len(), 1);
    }
}
