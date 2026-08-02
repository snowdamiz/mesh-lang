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
        .join("packages/mesh-binary")
}

#[test]
fn binary_package_enforces_canonical_reader_limits() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("binary-proof");
    let package = project.join(".mesh/packages/mesh-binary@0.1.0");
    fs::create_dir_all(package.join("binary")).unwrap();
    fs::copy(
        package_source().join("mesh.toml"),
        package.join("mesh.toml"),
    )
    .unwrap();
    fs::copy(
        package_source().join("binary/reader.mpl"),
        package.join("binary/reader.mpl"),
    )
    .unwrap();
    fs::write(
        project.join("mesh.toml"),
        "[package]\nname = \"binary-proof\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        project.join("main.mpl"),
        r##"
from Binary.Reader import BinaryError, BinaryReader, finish, read_fixed, read_u16_be, read_u8, read_vector, reader

fn decode(input :: Bytes) -> Int ! BinaryError do
  let state = reader(input, 10) ?
  case read_u8(state) do
    Err(error) -> Err(error)
    Ok((state, version)) -> decode_kind(state, version)
    Ok(_) -> Err(UnexpectedEnd)
  end
end

fn decode_kind(state :: BinaryReader, version :: Int) -> Int ! BinaryError do
  println("#{version}")
  case read_u16_be(state) do
    Err(error) -> Err(error)
    Ok((state, kind)) -> decode_payload(state, kind)
    Ok(_) -> Err(UnexpectedEnd)
  end
end

fn decode_payload(state :: BinaryReader, kind :: Int) -> Int ! BinaryError do
  println("#{kind}")
  case read_vector(state, 3) do
    Err(error) -> Err(error)
    Ok((state, payload)) -> decode_finish(state, payload)
    Ok(_) -> Err(UnexpectedEnd)
  end
end

fn decode_finish(state :: BinaryReader, payload :: Bytes) -> Int ! BinaryError do
  payload |> Bytes.to_hex() |> println()
  finish(state) ?
  Ok(0)
end

fn reject_trailing(input :: Bytes) -> Int ! BinaryError do
  let state = reader(input, 2) ?
  case read_u8(state) do
    Err(error) -> Err(error)
    Ok((state, _)) -> case finish(state) do
        Err(error) -> Err(error)
        Ok(_) -> Ok(0)
      end
    Ok(_) -> Err(UnexpectedEnd)
  end
end

fn reject_oversized_vector(input :: Bytes) -> Int ! BinaryError do
  let state = reader(input, Bytes.length(input)) ?
  case read_vector(state, 3) do
    Err(error) -> Err(error)
    Ok(_) -> Ok(0)
  end
end

fn reject_truncated_vector(input :: Bytes) -> Int ! BinaryError do
  let state = reader(input, Bytes.length(input)) ?
  case read_vector(state, 3) do
    Err(error) -> Err(error)
    Ok(_) -> Ok(0)
  end
end

fn reject_limit(input :: Bytes) -> Int ! BinaryError do
  let _ = reader(input, 1) ?
  Ok(0)
end

fn reject_invalid_length(input :: Bytes) -> Int ! BinaryError do
  let state = reader(input, 1) ?
  case read_fixed(state, -1) do
    Err(error) -> Err(error)
    Ok(_) -> Ok(0)
  end
end

fn proof() -> Int ! String do
  let valid = Bytes.from_hex("01234500000003aabbcc") ?
  case decode(valid) do
    Ok(_) -> println("valid")
    Err(_) -> println("unexpected-valid-error")
  end

  let trailing = Bytes.from_hex("0102") ?
  case reject_trailing(trailing) do
    Ok(_) -> println("unexpected-trailing-success")
    Err(_) -> println("trailing")
  end

  let oversized = Bytes.from_hex("00000004aabbccdd") ?
  case reject_oversized_vector(oversized) do
    Ok(_) -> println("unexpected-oversized-success")
    Err(_) -> println("oversized")
  end

  let truncated = Bytes.from_hex("00000003aabb") ?
  case reject_truncated_vector(truncated) do
    Ok(_) -> println("unexpected-truncated-success")
    Err(_) -> println("truncated")
  end

  case reject_limit(trailing) do
    Ok(_) -> println("unexpected-limit-success")
    Err(_) -> println("limit")
  end

  let one = Bytes.from_hex("00") ?
  case reject_invalid_length(one) do
    Ok(_) -> println("unexpected-invalid-success")
    Err(_) -> println("invalid")
  end
  Ok(0)
end

fn main() do
  case proof() do
    Ok(_) -> nil
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
    let run = Command::new(project.join("binary-proof")).output().unwrap();
    assert!(
        run.status.success(),
        "binary proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "1\n9029\naabbcc\nvalid\ntrailing\noversized\ntruncated\nlimit\ninvalid\n"
    );
}
