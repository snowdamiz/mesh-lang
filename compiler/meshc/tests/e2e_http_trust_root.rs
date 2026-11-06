#![cfg(unix)]

#[path = "support/test_artifacts.rs"]
mod artifacts;
#[path = "../../mesh-rt/tests/support/http_tls.rs"]
mod tls;

use std::net::TcpListener;
use std::process::Command;

#[test]
fn compiled_explicit_http_roots_verify_tls_and_stream_without_global_trust_changes() {
    let compiler = artifacts::meshc_bin();
    let temp = tempfile::tempdir().unwrap();
    let runtime = temp.path().join("libmesh_rt.a");
    std::fs::copy(compiler.parent().unwrap().join("libmesh_rt.a"), &runtime).unwrap();
    let fixture = tls::TlsFixture::new();
    let unrelated = tls::TlsFixture::new();
    for optimization in [0, 2] {
        let project = temp
            .path()
            .join(format!("http-explicit-roots-{optimization}"));
        std::fs::create_dir(&project).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let expired = TcpListener::bind("127.0.0.1:0").unwrap();
        let expired_port = expired.local_addr().unwrap().port();
        std::fs::write(project.join("root.pem"), fixture.pem("ca.pem")).unwrap();
        let root = project.join("root.pem").to_string_lossy().into_owned();
        let wrong = unrelated.path("ca.pem").to_string_lossy().into_owned();
        let private = fixture.path("ca.key").to_string_lossy().into_owned();
        let marker = project.join("stream.txt").to_string_lossy().into_owned();
        std::fs::write(
            project.join("mesh.toml"),
            "[package]\nname = \"http-explicit-roots\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(project.join("main.mpl"), format!(r##"
fn configured(url :: String, pem :: Bytes) -> Int do
  Http.build(:get, url)
    |> Http.trust_root_pem(pem)
    |> Http.timeout(2000)
    |> Http.max_response_bytes(16)
    |> Http.max_redirects(0)
end
fn rejected(request :: Int, prefix :: String) -> Bool do
  case Http.send(request) do
    Err(error) -> String.starts_with(error, prefix)
    Ok(_) -> false
  end
end
fn rejected_with(client :: Int, request :: Int) -> Bool do
  case Http.send_with(client, request) do
    Err(error) -> String.starts_with(error, "TLS_ERROR:")
    Ok(_) -> false
  end
end
fn success(response :: HttpResponse!String) -> Bool do
  case response do
    Ok(value) -> value.status == 200 && value.body == "ok"
    Err(_) -> false
  end
end
fn require(value :: Bool, label :: String) -> Result<(), String> do
  if value do Ok(nil) else Err(label) end
end
fn streamed(path :: String, remaining :: Int) -> Bool do
  let done = case File.read(path) do Ok(text) -> text == "6f6b" Err(_) -> false end
  if done do true
  else if remaining <= 0 do false
  else
    Timer.sleep(20)
    streamed(path, remaining - 1)
  end
end
fn proof() -> Result<(), String> do
  let roots = Bytes.from_utf8(File.read({root:?})?)
  let wrong = Bytes.from_utf8(File.read({wrong:?})?)
  let private = Bytes.from_utf8(File.read({private:?})?)
  let url = "https://localhost:{port}/"
  require(rejected(Http.build(:get, url) |> Http.timeout(2000), "TLS_ERROR:"), "default roots unexpectedly trusted local CA")?
  require(success(Http.send(configured(url, roots))), "selected root failed")?
  let client = Http.client()
  require(success(Http.send_with(client, configured(url, roots))), "selected root with client failed")?
  require(rejected_with(client, configured(url, wrong)), "wrong root reused a trusted connection")?
  require(rejected_with(client, Http.build(:get, url) |> Http.timeout(2000)), "selected root changed default client trust")?
  Http.client_close(client)
  require(rejected(configured("https://127.0.0.1:{port}/", roots), "TLS_ERROR:"), "hostname verification bypassed")?
  require(rejected(configured("https://localhost:{expired_port}/", roots), "TLS_ERROR:"), "expired certificate accepted")?
  require(rejected(configured(url, Bytes.empty()), "INVALID_TLS_ROOTS:"), "empty root did not poison request")?
  require(rejected(configured(url, private), "INVALID_TLS_ROOTS:"), "private key accepted as root")?
  require(rejected(configured("http://127.0.0.1:{port}/", roots), "INVALID_URL:"), "explicit roots allowed plaintext")?
  File.write({marker:?}, "")?
  let handle = Http.stream_bytes(configured(url, roots), fn(chunk) ->
    case File.append({marker:?}, Bytes.to_hex(chunk)) do Ok(_) -> "continue" Err(_) -> "stop" end
  end)
  require(handle > 0 && streamed({marker:?}, 100), "selected-root stream failed")?
  Http.cancel(handle)
  Ok(nil)
end
fn main() do
  case proof() do
    Ok(_) -> println("explicit-http-roots-ok")
    Err(error) -> println("failed:" <> error)
  end
end
"##)).unwrap();
        let output_path = project.join("client");
        let build = Command::new(&compiler)
            .args([
                "build",
                project.to_str().unwrap(),
                "--opt-level",
                &optimization.to_string(),
                "-o",
                output_path.to_str().unwrap(),
            ])
            .env("MESH_RT_LIB_PATH", &runtime)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "compile opt{optimization}: {}",
            String::from_utf8_lossy(&build.stderr)
        );
        let server = fixture.serve(listener, false, 7, false);
        let expired_server = fixture.serve(expired, true, 1, false);
        let run = Command::new(&output_path).output().unwrap();
        assert!(
            run.status.success(),
            "run opt{optimization}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "explicit-http-roots-ok\n",
            "opt{optimization}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(server.join().unwrap(), (7, 3), "opt{optimization}");
        assert_eq!(expired_server.join().unwrap(), (1, 0), "opt{optimization}");
    }
}
