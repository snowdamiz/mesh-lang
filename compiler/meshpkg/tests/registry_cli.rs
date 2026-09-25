//! meshpkg's registry commands against a registry served in the test:
//! search, named and manifest installs, login and publish, and what each
//! reports when the registry refuses or is not there.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

use flate2::write::GzEncoder;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn meshpkg_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_meshpkg"))
}

/// One request the registry received.
#[derive(Clone, Debug)]
struct Request {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

/// A registry answering `(method, target)` with a status and a body, and
/// recording every request.
struct Registry {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Registry {
    fn serve(routes: Vec<(&'static str, String, u16, Vec<u8>)>) -> Registry {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let target = parts.next().unwrap_or("").to_string();
                let mut headers = HashMap::new();
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
                    }
                }
                let length = headers
                    .get("content-length")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let (status, response) = routes
                    .iter()
                    .find(|(m, t, _, _)| *m == method && *t == target)
                    .map(|(_, _, status, body)| (*status, body.clone()))
                    .unwrap_or((404, b"not found".to_vec()));
                recorded.lock().unwrap().push(Request {
                    method,
                    target,
                    headers,
                    body,
                });
                let mut stream = &stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                );
                let _ = stream.write_all(&response);
            }
        });
        Registry { url, requests }
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

fn meshpkg(args: &[&str], dir: &Path, home: &Path) -> Output {
    Command::new(meshpkg_bin())
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A package tarball, as the registry serves it, and its SHA-256.
fn tarball() -> (Vec<u8>, String) {
    let mut bytes = Vec::new();
    {
        let mut archive = tar::Builder::new(GzEncoder::new(&mut bytes, Default::default()));
        for (path, contents) in [
            (
                "mesh.toml",
                "[package]\nname = \"acme/widget\"\nversion = \"1.2.0\"\n",
            ),
            ("widget.mpl", "pub fn size() -> Int do\n  3\nend\n"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, path, contents.as_bytes())
                .unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
    }
    let sha = format!("{:x}", Sha256::digest(&bytes));
    (bytes, sha)
}

#[test]
fn search_lists_what_the_registry_finds() {
    let home = tempfile::tempdir().unwrap();
    let found = json!([
        {"name": "acme/widget", "version": "1.2.0", "description": "Widgets"},
        {"name": "acme/gizmo", "version": "0.1.0", "description": "Gizmos for widgets"}
    ]);
    let registry = Registry::serve(vec![
        (
            "GET",
            "/api/v1/packages?search=widget".to_string(),
            200,
            found.to_string().into_bytes(),
        ),
        (
            "GET",
            "/api/v1/packages?search=none".to_string(),
            200,
            b"[]".to_vec(),
        ),
    ]);
    let dir = home.path();
    let output = meshpkg(&["search", "widget", "--registry", &registry.url], dir, dir);
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("acme/gizmo"), "{}", text(&output));
    let output = meshpkg(
        &["--json", "search", "widget", "--registry", &registry.url],
        dir,
        dir,
    );
    let listed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listed, found);
    let output = meshpkg(&["search", "none", "--registry", &registry.url], dir, dir);
    assert!(
        text(&output).contains("No packages found"),
        "{}",
        text(&output)
    );
    let output = meshpkg(
        &["--json", "search", "none", "--registry", &registry.url],
        dir,
        dir,
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "[]");
}

#[test]
fn install_fetches_verifies_and_locks_packages() {
    let (bytes, sha) = tarball();
    let registry = Registry::serve(vec![
        (
            "GET",
            "/api/v1/packages/acme/widget".to_string(),
            200,
            json!({"latest": {"version": "1.2.0", "sha256": sha}})
                .to_string()
                .into_bytes(),
        ),
        (
            "GET",
            "/api/v1/packages/acme/widget/1.2.0".to_string(),
            200,
            json!({"sha256": sha}).to_string().into_bytes(),
        ),
        (
            "GET",
            "/api/v1/packages/acme/widget/1.2.0/download".to_string(),
            200,
            bytes,
        ),
        (
            "GET",
            "/api/v1/packages/acme/broken".to_string(),
            200,
            json!({"latest": {"version": "1.2.0", "sha256": "0000"}})
                .to_string()
                .into_bytes(),
        ),
        (
            "GET",
            "/api/v1/packages/acme/broken/1.2.0/download".to_string(),
            200,
            b"not the package".to_vec(),
        ),
    ]);
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();

    // A named install downloads the latest release and locks it.
    let output = meshpkg(
        &["install", "acme/widget", "--registry", &registry.url],
        dir,
        home.path(),
    );
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("\"acme/widget\" = \"1.2.0\""),
        "{}",
        text(&output)
    );
    let installed = dir.join(".mesh/packages/acme/widget@1.2.0/widget.mpl");
    assert!(installed.is_file());
    let lock = std::fs::read_to_string(dir.join("mesh.lock")).unwrap();
    assert!(lock.contains(&sha), "{lock}");
    let output = meshpkg(
        &[
            "--json",
            "install",
            "acme/widget",
            "--registry",
            &registry.url,
        ],
        dir,
        home.path(),
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["manifest_changed"], false);

    // A download that does not match its checksum is refused.
    let output = meshpkg(
        &["install", "acme/broken", "--registry", &registry.url],
        dir,
        home.path(),
    );
    assert!(!output.status.success());
    assert!(
        text(&output).contains("SHA-256 mismatch"),
        "{}",
        text(&output)
    );

    // Installing what mesh.toml declares resolves each version once, then
    // installs from the lock.
    std::fs::remove_file(dir.join("mesh.lock")).unwrap();
    std::fs::write(
        dir.join("mesh.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"acme/widget\" = \"1.2.0\"\n",
    )
    .unwrap();
    let before = registry.requests().len();
    let output = meshpkg(&["install", "--registry", &registry.url], dir, home.path());
    assert!(output.status.success(), "{}", text(&output));
    assert!(
        text(&output).contains("Updated mesh.lock"),
        "{}",
        text(&output)
    );
    let targets: Vec<String> = registry.requests()[before..]
        .iter()
        .map(|request| request.target.clone())
        .collect();
    assert_eq!(
        targets,
        [
            "/api/v1/packages/acme/widget/1.2.0",
            "/api/v1/packages/acme/widget/1.2.0/download"
        ]
    );
    let before = registry.requests().len();
    let output = meshpkg(
        &["--json", "install", "--registry", &registry.url],
        dir,
        home.path(),
    );
    assert!(output.status.success(), "{}", text(&output));
    let targets: Vec<String> = registry.requests()[before..]
        .iter()
        .map(|request| request.target.clone())
        .collect();
    assert_eq!(targets, ["/api/v1/packages/acme/widget/1.2.0/download"]);

    // A path dependency is meshc's to fetch: install leaves it, and keeps
    // its lock entry. A download that does not match its lock is refused.
    std::fs::write(
        dir.join("mesh.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\"acme/widget\" = \"1.2.0\"\nlocal = { path = \"../local\" }\n",
    )
    .unwrap();
    let lock = std::fs::read_to_string(dir.join("mesh.lock")).unwrap();
    let path_entry =
        "\n[[packages]]\nname = \"local\"\nsource = \"../local\"\nrevision = \"local\"\n";
    std::fs::write(
        dir.join("mesh.lock"),
        format!("{}{path_entry}", lock.replace(&sha, &"0".repeat(64))),
    )
    .unwrap();
    let output = meshpkg(&["install", "--registry", &registry.url], dir, home.path());
    assert!(!output.status.success());
    assert!(
        text(&output).contains("SHA-256 mismatch for acme/widget@1.2.0"),
        "{}",
        text(&output)
    );
    std::fs::write(dir.join("mesh.lock"), format!("{lock}{path_entry}")).unwrap();
    let output = meshpkg(&["install", "--registry", &registry.url], dir, home.path());
    assert!(output.status.success(), "{}", text(&output));
    let lock = std::fs::read_to_string(dir.join("mesh.lock")).unwrap();
    assert!(
        lock.contains("name = \"local\"") && lock.contains(&sha),
        "{lock}"
    );

    // A registry that is not there.
    let output = meshpkg(
        &["install", "acme/widget", "--registry", "http://127.0.0.1:9"],
        dir,
        home.path(),
    );
    assert!(!output.status.success());
    assert!(
        text(&output).contains("Failed to query registry"),
        "{}",
        text(&output)
    );
}

#[test]
fn login_and_publish_upload_the_package_with_the_token() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    std::fs::write(
        dir.join("mesh.toml"),
        "[package]\nname = \"acme/widget\"\nversion = \"1.2.0\"\ndescription = \"Widgets\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("widget.mpl"),
        "pub fn size() -> Int do\n  3\nend\n",
    )
    .unwrap();

    let created = Registry::serve(vec![(
        "POST",
        "/api/v1/packages".to_string(),
        201,
        Vec::new(),
    )]);
    let output = meshpkg(&["publish", "--registry", &created.url], dir, home.path());
    assert!(!output.status.success());
    assert!(text(&output).contains("Not logged in"), "{}", text(&output));

    let output = meshpkg(&["login"], dir, home.path());
    assert!(!output.status.success());
    assert!(
        text(&output).contains("Token cannot be empty"),
        "{}",
        text(&output)
    );
    let output = meshpkg(&["login", "--token", "secret-token"], dir, home.path());
    assert!(output.status.success(), "{}", text(&output));
    let output = meshpkg(
        &["--json", "login", "--token", "secret-token"],
        dir,
        home.path(),
    );
    let saved: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(saved["status"], "ok");

    let output = meshpkg(
        &["--json", "publish", "--registry", &created.url],
        dir,
        home.path(),
    );
    assert!(output.status.success(), "{}", text(&output));
    let published: Value = serde_json::from_slice(&output.stdout).unwrap();
    let request = &created.requests()[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.headers["authorization"], "Bearer secret-token");
    assert_eq!(request.headers["x-package-name"], "acme/widget");
    assert_eq!(request.headers["x-package-version"], "1.2.0");
    assert_eq!(request.headers["x-package-description"], "Widgets");
    assert_eq!(
        published["sha256"],
        format!("{:x}", Sha256::digest(&request.body)).as_str()
    );
    let output = meshpkg(&["publish", "--registry", &created.url], dir, home.path());
    assert!(
        text(&output).contains("Published acme/widget@1.2.0"),
        "{}",
        text(&output)
    );

    for (status, message) in [
        (409, "already exists in registry"),
        (401, "Unauthorized"),
        (500, "Registry returned HTTP 500"),
    ] {
        let refusing = Registry::serve(vec![(
            "POST",
            "/api/v1/packages".to_string(),
            status,
            Vec::new(),
        )]);
        let output = meshpkg(&["publish", "--registry", &refusing.url], dir, home.path());
        assert!(!output.status.success());
        assert!(
            text(&output).contains(message),
            "{status}: {}",
            text(&output)
        );
        let output = meshpkg(
            &["--json", "publish", "--registry", &refusing.url],
            dir,
            home.path(),
        );
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert!(error["error"].as_str().unwrap().contains(message));
    }
}

/// A native package publishes its bindings and its archives, each archive
/// checked against the SHA-256 the manifest declares; a member that is
/// missing, a directory, reached through a link, or does not match is
/// refused before anything is uploaded.
#[test]
fn publish_packs_native_members_and_refuses_bad_ones() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let dir = project.path();
    let archive = b"!<arch>\nnot really an archive\n";
    let archive_sha = format!("{:x}", Sha256::digest(archive));
    std::fs::create_dir_all(dir.join("bindings")).unwrap();
    std::fs::create_dir_all(dir.join("native/x86_64-unknown-linux-gnu")).unwrap();
    std::fs::write(
        dir.join("bindings/math.mpl"),
        "@native(\"math_add\")\npub fn add(a :: Int, b :: Int) -> Int\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("native/x86_64-unknown-linux-gnu/libmath.a"),
        archive,
    )
    .unwrap();
    let manifest = |binding: &str, sha: &str| {
        format!(
            "[package]\nname = \"acme/math\"\nversion = \"0.1.0\"\n\n[native]\nabi = 1\nbindings = [\"{binding}\"]\n\n[[native.libraries]]\ntarget = \"x86_64-unknown-linux-gnu\"\npath = \"native/x86_64-unknown-linux-gnu/libmath.a\"\nsha256 = \"{sha}\"\n"
        )
    };
    let login = meshpkg(&["login", "--token", "secret-token"], dir, home.path());
    assert!(login.status.success(), "{}", text(&login));

    std::fs::write(
        dir.join("mesh.toml"),
        manifest("bindings/math.mpl", &archive_sha),
    )
    .unwrap();
    let registry = Registry::serve(vec![(
        "POST",
        "/api/v1/packages".to_string(),
        201,
        Vec::new(),
    )]);
    let output = meshpkg(&["publish", "--registry", &registry.url], dir, home.path());
    assert!(output.status.success(), "{}", text(&output));
    let body = registry.requests()[0].body.clone();
    let mut members: Vec<String> = tar::Archive::new(flate2::read::GzDecoder::new(body.as_slice()))
        .entries()
        .unwrap()
        .map(|entry| entry.unwrap().path().unwrap().display().to_string())
        .collect();
    members.sort();
    assert!(
        members.contains(&"bindings/math.mpl".to_string())
            && members.contains(&"native/x86_64-unknown-linux-gnu/libmath.a".to_string()),
        "{members:?}"
    );

    // Publishing refuses any link among the package's sources; one under a
    // hidden directory, which that walk skips, reaches the declared-member
    // check.
    std::fs::create_dir_all(dir.join(".linked")).unwrap();
    std::fs::create_dir_all(dir.join("bindings/dir.mpl")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.join("bindings/math.mpl"), dir.join(".linked/math.mpl"))
        .unwrap();
    let wrong_sha = "0".repeat(64);
    let mut refusals = vec![
        (
            manifest("bindings/math.mpl", &wrong_sha),
            "SHA-256 mismatch for native archive",
        ),
        (
            manifest("bindings/missing.mpl", &archive_sha),
            "does not exist or cannot be read",
        ),
        (
            manifest("bindings/dir.mpl", &archive_sha),
            "must be a file inside",
        ),
    ];
    if cfg!(unix) {
        refusals.push((
            manifest(".linked/math.mpl", &archive_sha),
            "must not contain a symbolic link",
        ));
    }
    for (manifest, message) in refusals {
        std::fs::write(dir.join("mesh.toml"), &manifest).unwrap();
        let output = meshpkg(&["publish", "--registry", &registry.url], dir, home.path());
        assert!(!output.status.success(), "{message}");
        assert!(
            text(&output).contains(message),
            "{message}: {}",
            text(&output)
        );
    }
    assert_eq!(
        registry.requests().len(),
        1,
        "a refused package was uploaded"
    );
}
