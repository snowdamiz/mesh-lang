use super::*;
use crate::actor::{Priority, ProcessId};
use crate::secret::destroy_owned;

#[test]
fn hmac_sha512_tag_rfc4231_binary_and_long_keys() {
    for (key, message, expected) in [
        (
            vec![0x0b; 20],
            b"Hi There".to_vec(),
            "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cde\
            daa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854",
        ),
        (
            vec![0xaa; 20],
            vec![0xdd; 50],
            "fa73b0089d56a284efb0f0756c890be9b1b5dbdd8ee81a3655f83e33b2279d39\
            bf3e848279a722c806b485a47e67c807b946a337bee8942674278859e13292fb",
        ),
        (
            vec![0xaa; 131],
            b"Test Using Larger Than Block-Size Key - Hash Key First".to_vec(),
            "80b24263c7c1a3ebb71493c1dd7be8b49b46d1f41b4aeec1121b013783f8f352\
            6b56d037e05f2598bd0fd2215d6a1e5295e64f73f63f0aec8b915a985d786598",
        ),
        // Boundary vectors independently computed with Python's hmac/hashlib.
        (
            vec![b'a'; 65536],
            vec![],
            "07a6ceb8ced338215998019ef389eaaafaef175031cb5969957433e2b3c064075ab\
            42e7e30fcdeefa18d622e47add7cd59dbb227eec8ca913ba878488765bfd4",
        ),
        (
            vec![b'a'],
            vec![b'b'; 65536],
            "ea0a4a1dbd374c4e3bcb33fe488fadac02da1070c1ca1cfbe7d4cd1cd3eed50e6\
            99f64a6dec4e3a8896c8088a4667896aa37464b79f35192e6be41d573958aa5",
        ),
    ] {
        let mut tag = [0; 64];
        SystemProvider
            .hmac_sha512(&key, &message, &mut tag)
            .unwrap();
        assert_eq!(&tag[..], decode_hex(expected).unwrap());
    }
}

#[test]
fn hmac_sha512_tag_provider_clears_output_on_bound_failure() {
    let oversized = vec![0; 65537];
    for (key, message) in [
        (&oversized[..], &b"message"[..]),
        (&b"key"[..], &oversized[..]),
    ] {
        let mut tag = [0xaa; 64];
        assert_eq!(
            SystemProvider.hmac_sha512(key, message, &mut tag),
            Err(ProviderError::InvalidLength)
        );
        assert_eq!(tag, [0; 64]);
    }
}

#[test]
fn hmac_sha512_tag_borrows_key_checks_owner_kind_lifetime_and_message_bounds() {
    crate::mesh_rt_init();
    let owner = ProcessId(78_512);
    let mut process = Process::new(owner, Priority::Normal);
    let key = insert_owned_resource(
        &mut process,
        ResourceKind::SecretBytes,
        Zeroizing::new(vec![0x0b; 20].into_boxed_slice()),
    )
    .unwrap();
    let bytes = b"Hi There";
    let message = mesh_bytes_new(bytes.as_ptr(), bytes.len() as u64);
    let first = hmac_sha512_tag_for_process(&process, &SystemProvider, key, message).unwrap();
    let second = hmac_sha512_tag_for_process(&process, &SystemProvider, key, message).unwrap();
    assert_eq!(&first[..], &second[..]);
    assert_eq!(
        &first[..],
        decode_hex(
            "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cde\
        daa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854"
        )
        .unwrap()
    );
    let foreign = Process::new(ProcessId(78_513), Priority::Normal);
    assert!(matches!(
        hmac_sha512_tag_for_process(&foreign, &SystemProvider, key, message),
        Err(CryptoFailure {
            tag: CryptoErrorTag::SecretDestroyed,
            ..
        })
    ));
    let wrong_kind = insert_owned_resource(
        &mut process,
        ResourceKind::AeadKey,
        Zeroizing::new(vec![0x42; 32].into_boxed_slice()),
    )
    .unwrap();
    assert!(matches!(
        hmac_sha512_tag_for_process(&process, &SystemProvider, wrong_kind, message),
        Err(CryptoFailure {
            tag: CryptoErrorTag::InvalidKey,
            ..
        })
    ));
    let oversized = vec![0; 65537];
    assert!(matches!(
        hmac_sha512_tag_for_process(
            &process,
            &SystemProvider,
            key,
            mesh_bytes_new(oversized.as_ptr(), oversized.len() as u64)
        ),
        Err(CryptoFailure {
            tag: CryptoErrorTag::InvalidLength,
            expected: 65536,
            actual: 65537
        })
    ));
    assert!(matches!(
        hmac_sha512_tag_for_process(&process, &SystemProvider, key, ptr::null()),
        Err(CryptoFailure {
            tag: CryptoErrorTag::InvalidLength,
            ..
        })
    ));
    destroy_owned(owner);
    assert!(matches!(
        hmac_sha512_tag_for_process(&process, &SystemProvider, key, message),
        Err(CryptoFailure {
            tag: CryptoErrorTag::SecretDestroyed,
            ..
        })
    ));
}
