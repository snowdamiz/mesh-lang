#![cfg(unix)]

#[path = "support/test_artifacts.rs"]
mod artifacts;
#[path = "../../mesh-rt/tests/support/http_persistence.rs"]
mod peer;

use std::process::Command;

fn check(opt_level: &str) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("persistence");
    std::fs::create_dir(&project).unwrap();
    std::fs::copy(
        artifacts::repo_root().join("tests/e2e/http_response_persistence.mpl"),
        project.join("main.mpl"),
    )
    .unwrap();
    let executable = project.join("client");
    let build = Command::new(artifacts::meshc_bin())
        .arg("build")
        .arg(&project)
        .args(["--opt-level", opt_level, "--output"])
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        artifacts::command_output_text(&build)
    );
    for peer in [
        peer::Peer::Http10,
        peer::Peer::Http10KeepAlive,
        peer::Peer::Http11,
        peer::Peer::Http11Close,
    ] {
        let (url, server) = peer::serve(peer);
        let run = Command::new(&executable)
            .env("MESH_HTTP_PERSISTENCE_URL", url)
            .output()
            .unwrap();
        let seen = server.join().unwrap();
        assert!(
            run.status.success(),
            "{}",
            artifacts::command_output_text(&run)
        );
        assert_eq!(
            String::from_utf8(run.stdout).unwrap(),
            "200:one\n200:two\n",
            "{seen:?}"
        );
        peer::verify(peer, &seen);
    }
}

#[test]
fn public_http_response_persistence_opt0() {
    check("0");
}

#[test]
fn public_http_response_persistence_opt2() {
    check("2");
}
