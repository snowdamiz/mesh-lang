#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn meshc_bin() -> PathBuf {
    let mut path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.join("meshc")
}

fn package_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("packages/mesh-anchor")
}

#[test]
fn anchor_package_validates_discriminator_owner_and_layout_version() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("anchor-proof");
    let package = project.join(".mesh/packages/mesh-anchor@0.1.0");
    fs::create_dir_all(package.join("anchor")).unwrap();
    fs::copy(
        package_source().join("mesh.toml"),
        package.join("mesh.toml"),
    )
    .unwrap();
    fs::copy(
        package_source().join("anchor/validator.mpl"),
        package.join("anchor/validator.mpl"),
    )
    .unwrap();
    fs::write(
        project.join("mesh.toml"),
        "[package]\nname = \"anchor-proof\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        project.join("main.mpl"),
        r##"
from Anchor.Validator import AccountLayout, account_payload, discriminator, versioned_payload

fn show(result :: Bytes!String) do
  case result do
    Ok(value) -> value |> Bytes.to_hex() |> println()
    Err(error) -> println(error)
  end
end

fn proof() -> Int!String do
  let owner = ("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f" |> Bytes.from_hex())?
  let other_owner = ("010102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f" |> Bytes.from_hex())?
  let prefix = ("State" |> discriminator())?
  prefix |> Bytes.to_hex() |> println()
  let payload = ("012a00" |> Bytes.from_hex())?
  let account = (payload |2> Bytes.concat(prefix))?
  show(account |> account_payload(owner, owner, "State"))
  show(account |> account_payload(owner, other_owner, "State"))
  show(payload |> account_payload(owner, owner, "State"))
  let wrong_account = (payload |2> Bytes.concat(("Pool" |> discriminator())?))?
  show(wrong_account |> account_payload(owner, owner, "State"))
  show(account |> versioned_payload(owner, owner, AccountLayout {
    account_name: "State",
    version_offset: 0,
    version: 1,
    minimum_payload_bytes: 3
  }))
  show(account |> versioned_payload(owner, owner, AccountLayout {
    account_name: "State",
    version_offset: 0,
    version: 2,
    minimum_payload_bytes: 3
  }))
  Ok(0)
end

fn main() do
  case proof() do
    Ok(_) -> println("done")
    Err(error) -> println(error)
  end
end
"##,
    )
    .unwrap();

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join("anchor-proof")).output().unwrap();
    assert!(
        run.status.success(),
        "Anchor proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "d8926b5e684bb6b1\n012a00\nANCHOR_OWNER: account owner mismatch\nANCHOR_DISCRIMINATOR: account data is shorter than 8 bytes\nANCHOR_DISCRIMINATOR: account discriminator mismatch\n012a00\nANCHOR_VERSION: expected 2 at payload offset 0, got 1\ndone\n"
    );
}
