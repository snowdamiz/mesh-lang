//! Synthetic public-vector compatibility checks for replacing the ML-KEM backend.
//! No private key is exported through Mesh; OpenSSL receives only known fixture seeds.
use super::provider::FixedProvider;
use super::{mlkem_decapsulate_material, mlkem_encapsulate_material, mlkem_public_key};
use crate::secret::CryptoErrorTag;
use std::{fs, process::Command};

fn unhex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn hex(value: &[u8]) -> String {
    value.iter().map(|b| format!("{b:02x}")).collect()
}
fn nist() -> serde_json::Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/vectors/mlkem/mlkem768-keygen-acvp-tc26.json"
    )))
    .unwrap()
}
fn seed_fixture() -> [u8; 64] {
    let v = nist();
    let seed =
        unhex(&(v["input"]["d"].as_str().unwrap().to_owned() + v["input"]["z"].as_str().unwrap()));
    seed.try_into().unwrap()
}

#[test]
fn c6_nist_acvp_mlkem768_keygen_tc26_matches_public_key() {
    let v = nist();
    assert_eq!(v["source"]["test_case_id"], 26);
    assert_eq!(
        hex(&mlkem_public_key(&seed_fixture())),
        v["expected"]["public_key"].as_str().unwrap()
    );
}

#[test]
fn c6_validation_precedes_entropy_and_fixed_size_rejection_remains_implicit() {
    let seed = seed_fixture();
    let public = mlkem_public_key(&seed);
    let invalid = [0xff; 1184];
    let error = mlkem_encapsulate_material(&FixedProvider::entropy_failure(), &invalid)
        .err()
        .unwrap();
    assert_eq!(error.tag, CryptoErrorTag::InvalidPublicKey);
    for coefficient in [0, 1, 255, 256, 511, 767] {
        // FIPS 203 ByteDecode_12 rejects coefficients >= q=3329. Exercise
        // both packed halves and every rank, retaining otherwise valid bytes.
        for invalid_coefficient in [3329_u16, 4095] {
            let mut malformed: [u8; 1184] = public.as_slice().try_into().unwrap();
            let start = 3 * (coefficient / 2);
            if coefficient % 2 == 0 {
                malformed[start] = invalid_coefficient as u8;
                malformed[start + 1] =
                    (malformed[start + 1] & 0xf0) | (invalid_coefficient >> 8) as u8;
            } else {
                malformed[start + 1] =
                    (malformed[start + 1] & 0x0f) | ((invalid_coefficient & 0x0f) << 4) as u8;
                malformed[start + 2] = (invalid_coefficient >> 4) as u8;
            }
            assert_eq!(
                mlkem_encapsulate_material(&FixedProvider::entropy_failure(), &malformed)
                    .err()
                    .unwrap()
                    .tag,
                CryptoErrorTag::InvalidPublicKey
            );
        }
    }
    let error = mlkem_encapsulate_material(
        &FixedProvider::entropy_failure(),
        public.as_slice().try_into().unwrap(),
    )
    .err()
    .unwrap();
    assert_eq!(error.tag, CryptoErrorTag::EntropyUnavailable);
    let (ciphertext, shared) = mlkem_encapsulate_material(
        &FixedProvider::with_random(&[0x73; 32]),
        public.as_slice().try_into().unwrap(),
    )
    .unwrap();
    for position in [0, 501, 1087] {
        let mut changed: [u8; 1088] = ciphertext.as_slice().try_into().unwrap();
        changed[position] ^= 1;
        let rejected = mlkem_decapsulate_material(&seed, &changed);
        assert_eq!(rejected.len(), 32);
        assert_ne!(rejected.as_ref(), shared.as_ref());
        assert_eq!(
            rejected.as_ref(),
            mlkem_decapsulate_material(&seed, &changed).as_ref()
        );
    }
}

#[test]
#[ignore = "requires OpenSSL >=3.5; run scripts/verify-mlkem-openssl.sh"]
fn c6_openssl_mlkem768_deterministic_interoperability_and_implicit_rejection() {
    let bin = std::env::var_os("MESH_OPENSSL_BIN").unwrap_or_else(|| "openssl".into());
    let temp = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let output = Command::new(&bin)
            .args(args)
            .current_dir(temp.path())
            .output()
            .expect("OpenSSL >=3.5 required, select with MESH_OPENSSL_BIN");
        assert!(
            output.status.success(),
            "OpenSSL failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    let version = run(&["version"]);
    println!("C6_OPENSSL={}", String::from_utf8_lossy(&version).trim());
    for seed in [seed_fixture(), [0; 64], std::array::from_fn(|i| i as u8)] {
        let seed_hex = format!("hexseed:{}", hex(&seed));
        run(&[
            "genpkey",
            "-algorithm",
            "ML-KEM-768",
            "-pkeyopt",
            &seed_hex,
            "-out",
            "private.pem",
        ]);
        run(&[
            "pkey",
            "-in",
            "private.pem",
            "-pubout",
            "-outform",
            "DER",
            "-out",
            "public.der",
        ]);
        let public = mlkem_public_key(&seed);
        let spki = fs::read(temp.path().join("public.der")).unwrap();
        let prefix = unhex("308204b2300b0609608648016503040402038204a100");
        assert_eq!(spki.len(), prefix.len() + 1184);
        assert_eq!(&spki[..prefix.len()], prefix.as_slice());
        assert_eq!(&spki[prefix.len()..], public.as_slice());
        let entropy = [0x73; 32];
        let ikme = format!("hexikme:{}", hex(&entropy));
        run(&[
            "pkeyutl",
            "-encap",
            "-pubin",
            "-inkey",
            "public.der",
            "-pkeyopt",
            &ikme,
            "-out",
            "ciphertext.bin",
            "-secret",
            "sender.bin",
        ]);
        let (ciphertext, shared) = mlkem_encapsulate_material(
            &FixedProvider::with_random(&entropy),
            public.as_slice().try_into().unwrap(),
        )
        .unwrap();
        assert_eq!(
            fs::read(temp.path().join("ciphertext.bin")).unwrap(),
            ciphertext
        );
        assert_eq!(
            fs::read(temp.path().join("sender.bin")).unwrap().as_slice(),
            shared.as_ref()
        );
        assert_eq!(
            mlkem_decapsulate_material(&seed, ciphertext.as_slice().try_into().unwrap()).as_ref(),
            shared.as_ref()
        );
        // Decapsulation of an invalid full-length ciphertext must not become an
        // error oracle. Match OpenSSL's exact implicit-rejection secret instead.
        let mut modified: [u8; 1088] = ciphertext.as_slice().try_into().unwrap();
        modified[1087] ^= 1;
        fs::write(temp.path().join("modified.bin"), modified).unwrap();
        run(&[
            "pkeyutl",
            "-decap",
            "-inkey",
            "private.pem",
            "-in",
            "modified.bin",
            "-secret",
            "rejected.bin",
        ]);
        let rejected = mlkem_decapsulate_material(&seed, &modified);
        assert_eq!(
            fs::read(temp.path().join("rejected.bin"))
                .unwrap()
                .as_slice(),
            rejected.as_ref()
        );
        assert_ne!(rejected.as_ref(), shared.as_ref());
    }
    println!("C6_OPENSSL_INTEROP=passed");
}
