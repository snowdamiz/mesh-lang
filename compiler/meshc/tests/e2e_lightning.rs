#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::Command;

#[path = "support/test_artifacts.rs"]
mod artifacts;

#[test]
fn public_lightning_package_verifies_all_bolt_vectors_in_debug_and_optimized_code() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let vectors: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../tests/vectors/lightning/invoice-test.json"
    ))
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("lightning-proof");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("mesh.toml"), format!("[package]\nname = \"lightning-proof\"\nversion = \"0.1.0\"\n[dependencies]\nmesh-lightning = {{ path = {:?} }}\n", root.join("packages/mesh-lightning"))).unwrap();
    let mut source = String::from("from Lightning.Invoice import parse_invoice, fixed_invoice_amount, invoice_expires_at\nfn check() -> Bool!String do\n");
    for value in &vectors {
        source.push_str(&format!(
            "  assert(Result.is_ok(parse_invoice({})) == {})\n",
            serde_json::to_string(value["invoice"].as_str().unwrap()).unwrap(),
            value["valid"]
        ));
    }
    source.push_str(&format!(
        "  let coffee = parse_invoice({})?\n",
        serde_json::to_string(vectors[1]["invoice"].as_str().unwrap()).unwrap()
    ));
    source.push_str("  assert(fixed_invoice_amount(coffee)? == U64.parse(\"250000000\")?)\n  assert(invoice_expires_at(coffee)? == U64.parse(\"1496314718\")?)\n  assert(coffee.network == 0)\n  assert(!coffee.has_fallback)\n  assert(Bytes.to_hex(coffee.payee) == \"03e7156ae33b0a208d0744199163177e909e80176e55d97a2f221ede0f934dd9ad\")\n  assert(Bytes.secure_equals(coffee.description_hash, Crypto.sha256(Bytes.from_utf8(\"1 cup coffee\"))))\n");
    source.push_str(&format!("  let donation = parse_invoice({})?\n  assert(Result.is_err(fixed_invoice_amount(donation)))\n", serde_json::to_string(vectors[0]["invoice"].as_str().unwrap()).unwrap()));
    source.push_str("  let decode = Lightning.invoice_details\n  assert(Result.is_err(decode(\"invalid\")))\n  Ok(true)\nend\nfn main() do\n  assert(check() == Ok(true))\n  println(\"lightning vectors verified\")\nend\n");
    source = source.replace("assert(", "ensure(");
    source.push_str("\nfn ensure(value :: Bool) do\n  if !value do panic(\"lightning assertion failed\") end\nend\n");
    fs::write(project.join("main.mpl"), source).unwrap();
    let compiler = artifacts::meshc_bin();
    for level in ["0", "2"] {
        let output = temp.path().join(format!("lightning-opt{level}"));
        let build = Command::new(&compiler)
            .arg("build")
            .arg(&project)
            .args(["--opt-level", level, "-o"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            artifacts::command_output_text(&build)
        );
        let run = Command::new(&output).output().unwrap();
        assert!(
            run.status.success(),
            "{}",
            artifacts::command_output_text(&run)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            "lightning vectors verified"
        );
    }
}

#[test]
fn random_only_preimage_resource_preserves_commitment_through_sealed_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("preimage-proof");
    fs::create_dir(&project).unwrap();
    fs::write(
        project.join("main.mpl"),
        include_str!("../../../tests/e2e/hash_preimage.mpl"),
    )
    .unwrap();
    let compiler = artifacts::meshc_bin();
    for level in ["0", "2"] {
        let output = temp.path().join(format!("preimage-opt{level}"));
        let build = Command::new(&compiler)
            .arg("build")
            .arg(&project)
            .args(["--opt-level", level, "-o"])
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            artifacts::command_output_text(&build)
        );
        let run = Command::new(&output).output().unwrap();
        assert!(
            run.status.success(),
            "{}",
            artifacts::command_output_text(&run)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout).trim(),
            "hash preimage verified"
        );
    }
}
