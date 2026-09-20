#![cfg(unix)]

use sha2::{Digest, Sha256};
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
        .join("packages/mesh-borsh")
}

#[test]
fn native_borsh_package_round_trips_and_rejects_malformed_input() {
    let binding_source = fs::read_to_string(package_source().join("bindings/borsh.mpl")).unwrap();
    let binding_parse = mesh_parser::parse(&binding_source);
    let binding_typeck = mesh_typeck::check(&binding_parse);
    assert!(
        binding_typeck.errors.is_empty(),
        "binding typecheck failed: {:#?}",
        binding_typeck.errors
    );
    let binding_exports = mesh_typeck::collect_exports(&binding_parse, &binding_typeck);
    assert_eq!(
        binding_exports.functions["writer"].ty.to_string(),
        "(Int) -> Result<Int, String>"
    );
    assert_eq!(
        binding_exports.functions["read_bool"].ty.to_string(),
        "(Int) -> Result<Bool, String>"
    );

    let temp = tempfile::tempdir().unwrap();
    let package = temp.path().join("mesh-borsh");
    let native = package.join("native");
    fs::create_dir_all(&native).unwrap();
    fs::create_dir(package.join("bindings")).unwrap();
    fs::copy(
        package_source().join("bindings/borsh.mpl"),
        package.join("bindings/borsh.mpl"),
    )
    .unwrap();
    fs::copy(
        package_source().join("native/lib.rs"),
        native.join("lib.rs"),
    )
    .unwrap();

    let archive = native.join("libmesh_borsh.a");
    let compile = Command::new("rustc")
        .args([
            "--crate-name",
            "mesh_borsh_native",
            "--crate-type",
            "staticlib",
            "--edition",
            "2021",
            "-O",
            native.join("lib.rs").to_str().unwrap(),
            "-o",
            archive.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "native Borsh build failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let target = mesh_codegen::link::effective_target_triple(None).unwrap();
    let sha256 = format!("{:x}", Sha256::digest(fs::read(&archive).unwrap()));
    fs::write(
        package.join("mesh.toml"),
        format!(
            r#"[package]
name = "mesh-borsh"
version = "0.1.0"

[native]
abi = 1
bindings = ["bindings/borsh.mpl"]

[[native.libraries]]
target = "{target}"
path = "native/libmesh_borsh.a"
sha256 = "{sha256}"
"#
        ),
    )
    .unwrap();

    let project = temp.path().join("borsh-proof");
    fs::create_dir(&project).unwrap();
    fs::write(
        project.join("mesh.toml"),
        "[package]\nname = \"borsh-proof\"\nversion = \"0.1.0\"\n\n[dependencies]\nmesh-borsh = { path = \"../mesh-borsh\" }\n",
    )
    .unwrap();
    fs::write(
        project.join("main.mpl"),
        r##"
from Bindings.Borsh import close_reader, finish_reader, finish_writer, read_bool, read_fixed, read_i128, read_i16, read_i32, read_i64, read_i8, read_len, read_option_tag, read_string, read_u128, read_u16, read_u32, read_u64, read_u8, read_vec, reader, remaining, write_bool, write_fixed, write_i128, write_i16, write_i32, write_i64, write_i8, write_len, write_option_tag, write_string, write_u128, write_u16, write_u32, write_u64, write_u8, write_vec, writer

struct Record do
  amount :: U64
  active :: Bool
end

fn decode_record(handle :: Int) -> Record!String do
  Ok(Record {
    amount: (read_u64(handle))?,
    active: (read_bool(handle))?
  })
end

fn encode() -> Bytes!String do
  let handle = (writer(1_024))?
  (handle |> write_u8(255))?
  (handle |> write_i8(-1))?
  (handle |> write_u16(513))?
  (handle |> write_i16(-513))?
  (handle |> write_u32(16_909_060))?
  (handle |> write_i32(-16_909_060))?
  (handle |> write_u64((U64.parse("18446744073709551615"))?))?
  (handle |> write_i64((I128.parse("-9223372036854775808"))?))?
  (handle |> write_u128((U128.parse("340282366920938463463374607431768211455"))?))?
  (handle |> write_i128((I128.parse("-170141183460469231731687303715884105728"))?))?
  (handle |> write_bool(true))?
  (handle |> write_fixed(("aabbcc" |> Bytes.from_hex())?))?
  (handle |> write_vec(("000102" |> Bytes.from_hex())?))?
  (handle |> write_string("solana"))?
  (handle |> write_option_tag(true))?
  (handle |> write_u16(42))?
  (handle |> write_len(2))?
  (handle |> write_u16(7))?
  (handle |> write_u16(8))?
  (handle |> write_u64((U64.parse("42"))?))?
  (handle |> write_bool(false))?
  finish_writer(handle)
end

fn decode(bytes :: Bytes) -> Int!String do
  let handle = (reader(bytes, 16))?
  println("#{(read_u8(handle))?}")
  println("#{(read_i8(handle))?}")
  println("#{(read_u16(handle))?}")
  println("#{(read_i16(handle))?}")
  println("#{(read_u32(handle))?}")
  println("#{(read_i32(handle))?}")
  (read_u64(handle))? |> U64.to_string() |> println()
  (read_i64(handle))? |> I128.to_string() |> println()
  (read_u128(handle))? |> U128.to_string() |> println()
  (read_i128(handle))? |> I128.to_string() |> println()
  println("#{(read_bool(handle))?}")
  (read_fixed(handle, 3))? |> Bytes.to_hex() |> println()
  (read_vec(handle))? |> Bytes.to_hex() |> println()
  println((read_string(handle))?)
  println("#{(read_option_tag(handle))?}:#{(read_u16(handle))?}")
  println("#{(read_len(handle))?}:#{(read_u16(handle))?}:#{(read_u16(handle))?}")
  let record = (decode_record(handle))?
  println("#{U64.to_string(record.amount)}:#{record.active}")
  println("#{(remaining(handle))?}")
  (finish_reader(handle))?
  Ok(0)
end

fn print_error(result :: Int!String) do
  case result do
    Ok(_) -> println("unexpected-ok")
    Err(error) -> println(error)
  end
end

fn malformed() do
  let short = (reader(Bytes.from_utf8("x"), 16))?
  print_error(read_u32(short))
  close_reader(short)
  let invalid_bool = (reader(("02" |> Bytes.from_hex())?, 16))?
  case read_bool(invalid_bool) do
    Ok(_) -> println("unexpected-ok")
    Err(error) -> println(error)
  end
  close_reader(invalid_bool)
  let oversized = (reader(("11000000" |> Bytes.from_hex())?, 16))?
  case read_vec(oversized) do
    Ok(_) -> println("unexpected-ok")
    Err(error) -> println(error)
  end
  close_reader(oversized)
  Ok(())
end

fn bounded_writer() do
  let handle = (writer(4))?
  case (handle |> write_vec(Bytes.from_utf8("x"))) do
    Ok(_) -> println("unexpected-ok")
    Err(error) -> println(error)
  end
  (finish_writer(handle))? |> Bytes.to_hex() |> println()
  Ok(())
end

fn main() do
  case encode() do
    Ok(bytes) -> case decode(bytes) do
      Ok(_) -> case malformed() do
        Ok(_) -> case bounded_writer() do
          Ok(_) -> println("done")
          Err(error) -> println(error)
        end
        Err(error) -> println(error)
      end
      Err(error) -> println(error)
    end
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
    let run = Command::new(project.join("borsh-proof"))
        .env("RUST_BACKTRACE", "1")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "Borsh proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "255\n-1\n513\n-513\n16909060\n-16909060\n18446744073709551615\n-9223372036854775808\n340282366920938463463374607431768211455\n-170141183460469231731687303715884105728\ntrue\naabbcc\n000102\nsolana\ntrue:42\n2:7:8\n42:false\n0\nBORSH_EOF: need 4 bytes at offset 0, only 1 remain\nBORSH_BOOL: expected 0 or 1, got 2\nBORSH_LIMIT: collection length 17 exceeds 16\nBORSH_LIMIT: output length 5 exceeds 4\n\ndone\n"
    );
}
