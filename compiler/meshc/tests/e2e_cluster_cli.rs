//! `meshc cluster ...` against a running node of the `meshc init --clustered`
//! scaffold: every inspection command in both output forms, the
//! authenticated control commands, and what it says when it cannot connect.

#[path = "support/test_artifacts.rs"]
mod test_artifacts;

use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use test_artifacts::{command_output_text, ensure_mesh_rt_staticlib, meshc_bin};

const COOKIE: &str = "cluster-cli-test-cookie";
const OPERATOR_KEY: &str = "cluster-cli-test-operator-key-0123456789";

/// A node that is killed when the test ends, pass or fail.
struct Node(Child);

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn meshc(args: &[&str], dir: &Path) -> Output {
    Command::new(meshc_bin())
        .args(args)
        .current_dir(dir)
        .env("MESH_CLUSTER_COOKIE", COOKIE)
        .env("MESH_OPERATOR_KEY", OPERATOR_KEY)
        .output()
        .expect("meshc runs")
}

fn cluster(args: &[&str], dir: &Path) -> String {
    let mut all = vec!["cluster"];
    all.extend_from_slice(args);
    let output = meshc(&all, dir);
    assert!(
        output.status.success(),
        "meshc {}:\n{}",
        all.join(" "),
        command_output_text(&output)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn cluster_json(args: &[&str], dir: &Path) -> Value {
    let mut all = args.to_vec();
    all.push("--json");
    let text = cluster(&all, dir);
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{error}: {text}"))
}

#[test]
fn cluster_commands_inspect_and_control_a_running_node() {
    ensure_mesh_rt_staticlib();
    let temp = tempfile::tempdir().unwrap();
    let init = meshc(&["init", "--clustered", "hello_cluster"], temp.path());
    assert!(init.status.success(), "{}", command_output_text(&init));
    let project = temp.path().join("hello_cluster");
    let build = meshc(&["build", "."], &project);
    assert!(build.status.success(), "{}", command_output_text(&build));

    let port = free_port();
    let target = format!("primary@127.0.0.1:{port}");
    let _node = Node(
        Command::new(project.join("output"))
            .current_dir(&project)
            .env("MESH_CLUSTER_COOKIE", COOKIE)
            .env("MESH_OPERATOR_KEY", OPERATOR_KEY)
            .env("MESH_NODE_NAME", &target)
            .env("MESH_DISCOVERY_SEED", "localhost")
            .env("MESH_CLUSTER_PORT", port.to_string())
            .env("MESH_CONTINUITY_ROLE", "primary")
            .env("MESH_CONTINUITY_PROMOTION_EPOCH", "0")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the node starts"),
    );

    // The node answers once it has bootstrapped.
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        let output = meshc(&["cluster", "status", &target, "--json"], &project);
        if output.status.success() {
            break serde_json::from_slice::<Value>(&output.stdout).unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "the node never answered:\n{}",
            command_output_text(&output)
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    assert!(status.to_string().contains(&target), "{status}");
    assert!(cluster(&["status", &target], &project).contains("cluster_role: primary"));

    // Every inspection command, as JSON and as text.
    for command in [
        "snapshot",
        "capacity",
        "pressure",
        "routing",
        "scaling",
        "diagnostics",
        "events",
        "continuity",
    ] {
        let json = cluster_json(&[command, &target], &project);
        assert!(json.is_object() || json.is_array(), "{command}: {json}");
        let text = cluster(&[command, &target], &project);
        assert!(!text.trim().is_empty(), "{command} printed nothing");
    }

    // The scaffold's `@cluster` work runs at startup and leaves a completed
    // continuity record.
    let deadline = Instant::now() + Duration::from_secs(30);
    let key = loop {
        let list = cluster_json(&["continuity", &target, "--limit", "5"], &project);
        if let Some(key) = list["records"][0]["request_key"].as_str() {
            break key.to_string();
        }
        assert!(Instant::now() < deadline, "no continuity record: {list}");
        std::thread::sleep(Duration::from_millis(250));
    };
    let record = cluster_json(&["continuity", &target, &key], &project);
    assert_eq!(record["record"]["phase"], "completed", "{record}");
    assert!(cluster(&["continuity", &target, &key], &project).contains("completed"));
    let explained = cluster_json(&["explain", &target, &key], &project);
    assert!(explained.to_string().contains(&key), "{explained}");
    assert!(cluster(&["explain", &target, &key], &project).contains(&key));

    // A key the node has never seen.
    for command in ["continuity", "explain"] {
        let unknown = meshc(&["cluster", command, &target, "no-such-key"], &project);
        assert!(!unknown.status.success());
        assert!(
            command_output_text(&unknown).contains("request_key_not_found"),
            "{}",
            command_output_text(&unknown)
        );
    }

    // Authenticated control: pausing and resuming the autoscaler, a manual
    // capacity override and drains go through the controller's consensus,
    // which a lone scaffold node does not run: each is refused, and says so.
    for control in [
        vec!["autoscale", "pause", target.as_str()],
        vec!["autoscale", "resume", target.as_str()],
        vec!["scale", target.as_str(), "2"],
        vec!["drain", target.as_str(), "worker-1"],
        vec!["cancel-drain", target.as_str(), "worker-1"],
    ] {
        let mut args = vec!["cluster"];
        args.extend(control.iter().copied());
        for json in [false, true] {
            let mut args = args.clone();
            if json {
                args.push("--json");
            }
            let output = meshc(&args, &project);
            assert!(!output.status.success(), "{}", command_output_text(&output));
            assert!(
                command_output_text(&output).contains("consensus_rpc_server_unavailable"),
                "{}",
                command_output_text(&output)
            );
        }
    }
}

#[test]
fn cluster_commands_explain_what_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let target = format!("primary@127.0.0.1:{}", free_port());

    // No cookie in the environment or a file.
    let output = Command::new(meshc_bin())
        .args(["cluster", "status", &target])
        .env_remove("MESH_CLUSTER_COOKIE")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        command_output_text(&output).contains("MESH_CLUSTER_COOKIE"),
        "{}",
        command_output_text(&output)
    );

    // A cookie file readable by others is refused.
    let cookie = dir.join("cookie");
    fs::write(&cookie, COOKIE).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cookie, fs::Permissions::from_mode(0o644)).unwrap();
        let output = meshc(
            &[
                "cluster",
                "status",
                &target,
                "--cookie-file",
                cookie.to_str().unwrap(),
            ],
            dir,
        );
        assert!(!output.status.success(), "{}", command_output_text(&output));
        fs::set_permissions(&cookie, fs::Permissions::from_mode(0o600)).unwrap();
    }

    // Nothing listens at the target.
    let output = meshc(
        &[
            "cluster",
            "status",
            &target,
            "--cookie-file",
            cookie.to_str().unwrap(),
            "--timeout-ms",
            "500",
        ],
        dir,
    );
    assert!(!output.status.success(), "{}", command_output_text(&output));

    // A control request with no operator key.
    let output = Command::new(meshc_bin())
        .args(["cluster", "autoscale", "pause", &target])
        .env("MESH_CLUSTER_COOKIE", COOKIE)
        .env_remove("MESH_OPERATOR_KEY")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        command_output_text(&output).contains("MESH_OPERATOR_KEY"),
        "{}",
        command_output_text(&output)
    );
}
