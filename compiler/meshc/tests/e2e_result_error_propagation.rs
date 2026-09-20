#![cfg(unix)]

#[path = "support/test_artifacts.rs"]
mod artifacts;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn imported_wide_nested_error_survives_mapping_and_try() {
    artifacts::ensure_mesh_rt_staticlib();

    let temp = tempfile::tempdir().expect("temporary project");
    let project = temp.path().join("result-error-propagation");
    let package = project.join(".mesh/packages/error-probe@0.1.0");
    fs::create_dir_all(package.join("errors")).expect("package source directory");
    fs::write(
        package.join("mesh.toml"),
        "[package]\nname = \"error-probe\"\nversion = \"0.1.0\"\n",
    )
    .expect("package manifest");
    fs::write(
        package.join("errors/verify.mpl"),
        r#"
pub type IdentityError do
  InvalidIdentity
  IdentityDetails(first :: Int, second :: Int, third :: Int, fourth :: Int)
end

pub type ProtocolError do
  InvalidProtocol
  ProtocolDetails(first :: Int, second :: Int, third :: Int, fourth :: Int)
end

pub type PrekeyError do
  CryptoFailure(error :: CryptoError)
  IdentityFailure(error :: IdentityError)
  ProtocolFailure(error :: ProtocolError)
  InvalidBundle
end

pub fn verify(expired :: Bool) -> Bool ! PrekeyError do
  if expired do
    Err(InvalidBundle)
  else
    Ok(true)
  end
end
"#,
    )
    .expect("package source");

    fs::write(
        project.join("mesh.toml"),
        "[package]\nname = \"result-error-propagation\"\nversion = \"0.1.0\"\n",
    )
    .expect("project manifest");
    fs::write(
        project.join("main.mpl"),
        r#"
from Errors.Verify import PrekeyError, verify

type SessionError do
  AuthenticationRejected
  CryptoFailure(error :: CryptoError)
  IdentityFailure(error :: PrekeyError)
  PrekeyFailure(error :: PrekeyError)
  ProtocolFailure(error :: PrekeyError)
  InvalidHandshake
end

fn wrapped(expired :: Bool) -> Bool ! SessionError do
  let valid = case verify(expired) do
    Err(error) -> Err(PrekeyFailure(error))
    Ok(value) -> Ok(value)
  end ?
  Ok(valid)
end

fn main() do
  case verify(true) do
    Err(_) -> println("direct:rejected")
    Ok(_) -> println("direct:accepted")
  end
  case wrapped(true) do
    Err(_) -> println("wrapped:rejected")
    Ok(_) -> println("wrapped:accepted")
  end
end
"#,
    )
    .expect("project source");

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().expect("project path")])
        .output()
        .expect("run meshc");
    assert!(
        build.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(project.join("result-error-propagation"))
        .output()
        .expect("run proof");
    assert!(run.status.success(), "proof failed: {run:?}");
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "direct:rejected\nwrapped:rejected\n"
    );
}

fn meshc_bin() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("test executable")
        .parent()
        .expect("test executable directory")
        .to_path_buf();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.join("meshc")
}
