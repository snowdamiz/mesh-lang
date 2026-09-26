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

/// A node that is stopped when the test ends, pass or fail.
struct Node(Child);

impl Drop for Node {
    fn drop(&mut self) {
        test_artifacts::stop_child(&mut self.0);
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

    // A key that is blank, or a key file that is not one.
    let refused = |extra: &[&str], key: &str, message: &str| {
        let mut args = vec!["cluster", "autoscale", "pause", target.as_str()];
        args.extend_from_slice(extra);
        let output = Command::new(meshc_bin())
            .args(&args)
            .env("MESH_CLUSTER_COOKIE", COOKIE)
            .env("MESH_OPERATOR_KEY", key)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}");
        assert!(
            command_output_text(&output).contains(message),
            "{args:?}: {}",
            command_output_text(&output)
        );
    };
    refused(&[], "  ", "MESH_OPERATOR_KEY must not be blank");
    let key_file = |name: &str| dir.join(name).to_str().unwrap().to_string();
    refused(
        &["--operator-key-file", &key_file("missing")],
        OPERATOR_KEY,
        "cannot inspect operator key file",
    );
    refused(
        &["--operator-key-file", dir.to_str().unwrap()],
        OPERATOR_KEY,
        "must be a regular file",
    );
    fs::write(dir.join("blank"), "\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir.join("blank"), fs::Permissions::from_mode(0o600)).unwrap();
        refused(
            &["--operator-key-file", &key_file("blank")],
            OPERATOR_KEY,
            "blank must not be blank",
        );
        // Owner-only yet unreadable (permissions do not bind a privileged user).
        fs::write(dir.join("sealed"), OPERATOR_KEY).unwrap();
        fs::set_permissions(dir.join("sealed"), fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(dir.join("sealed")).is_err() {
            refused(
                &["--operator-key-file", &key_file("sealed")],
                OPERATOR_KEY,
                "cannot read operator key file",
            );
        }
        fs::set_permissions(dir.join("sealed"), fs::Permissions::from_mode(0o600)).unwrap();
    }

    // One record by key, or a list up to a limit: not both.
    let output = meshc(
        &["cluster", "continuity", &target, "some-key", "--limit", "5"],
        dir,
    );
    assert!(!output.status.success());
    assert!(
        command_output_text(&output).contains("does not accept --limit when request_key"),
        "{}",
        command_output_text(&output)
    );
}

/// The Docker proof topology, torn down however the test ends: its
/// controller stopped before the capacity it made is removed, as the
/// proof's own cleanup does.
struct Topology {
    compose_file: String,
    cluster_id: String,
    environment: Vec<(String, String)>,
}

impl Topology {
    fn docker(&self, args: &[&str]) -> Output {
        Command::new("docker")
            .args(args)
            .envs(self.environment.iter().cloned())
            .output()
            .expect("docker runs")
    }

    fn remove_managed_capacity(&self) {
        let cluster = format!("label=mesh.cluster={}", self.cluster_id);
        let listed = self.docker(&[
            "ps",
            "-aq",
            "--filter",
            &cluster,
            "--filter",
            "label=mesh.managed=true",
        ]);
        let ids = String::from_utf8_lossy(&listed.stdout).to_string();
        let ids: Vec<&str> = ids.lines().filter(|id| !id.is_empty()).collect();
        if !ids.is_empty() {
            let mut args = vec!["rm", "-f"];
            args.extend(ids);
            self.docker(&args);
        }
    }
}

impl Drop for Topology {
    fn drop(&mut self) {
        let compose = |args: &[&str]| {
            let mut all = vec!["compose", "-f", self.compose_file.as_str()];
            all.extend_from_slice(args);
            self.docker(&all);
        };
        compose(&["stop", "--timeout", "10"]);
        self.remove_managed_capacity();
        compose(&["down", "--volumes", "--remove-orphans", "--timeout", "10"]);
        self.remove_managed_capacity();
    }
}

/// Every command against the Docker proof's autonomous cluster, whose
/// controllers run consensus: the snapshot names the leader, and pausing
/// and resuming the autoscaler, a capacity override and a drain are
/// accepted and reported.
#[test]
#[ignore = "requires Docker (the coverage run starts it)"]
fn cluster_commands_steer_an_autonomous_cluster() {
    let temp = tempfile::tempdir().unwrap();
    let connection = temp.path().join("connection.json");
    let start = Command::new(meshc_bin())
        .args([
            "proof",
            "docker-autoscaling",
            "--start-only",
            "--keep-running",
        ])
        .arg("--connection-file")
        .arg(&connection)
        .arg("--evidence-dir")
        .arg(temp.path().join("evidence"))
        .output()
        .expect("meshc runs");
    let manifest: Value = fs::read(&connection)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| panic!("no connection manifest:\n{}", command_output_text(&start)));
    let environment: Vec<(String, String)> = manifest["environment"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| (name.clone(), value.as_str().unwrap().to_string()))
        .collect();
    let _topology = Topology {
        compose_file: manifest["docker"]["composeFile"]
            .as_str()
            .unwrap()
            .to_string(),
        cluster_id: manifest["clusterId"].as_str().unwrap().to_string(),
        environment: environment.clone(),
    };
    assert!(start.status.success(), "{}", command_output_text(&start));

    let cluster_id = manifest["clusterId"].as_str().unwrap();
    let cookie_file = manifest["cookieFile"].as_str().unwrap();
    let key_file = manifest["operatorKeyFile"].as_str().unwrap();
    let run = |args: &[&str]| {
        let output = Command::new(meshc_bin())
            .arg("cluster")
            .args(args)
            .args(["--cookie-file", cookie_file])
            .envs(environment.iter().cloned())
            .env_remove("MESH_CLUSTER_COOKIE")
            .output()
            .expect("meshc runs");
        let text = if output.status.success() {
            String::from_utf8_lossy(&output.stdout).into_owned()
        } else {
            command_output_text(&output)
        };
        (output.status.success(), text)
    };

    // The controller that leads consensus.
    let targets: Vec<&str> = manifest["controllerTargets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|target| target.as_str().unwrap())
        .collect();
    let deadline = Instant::now() + Duration::from_secs(60);
    let leader = loop {
        let leader = targets.iter().copied().find(|target| {
            let (ok, text) = run(&["snapshot", target, "--json"]);
            ok && serde_json::from_str::<Value>(&text)
                .is_ok_and(|snapshot| snapshot["consensus"]["state"] == "leader")
        });
        if let Some(leader) = leader {
            break leader;
        }
        assert!(Instant::now() < deadline, "no controller leads");
        std::thread::sleep(Duration::from_millis(500));
    };

    // The topology is ready while the reconciler may still drain the extra
    // worker its fault injection makes: let it finish.
    let deadline = Instant::now() + Duration::from_secs(120);
    while {
        let (ok, text) = run(&["snapshot", leader, "--json"]);
        let snapshot: Value = serde_json::from_str(&text).unwrap_or_default();
        !ok || snapshot["autonomous"]["last_reconcile"]["drains"]
            .as_array()
            .is_none_or(|drains| !drains.is_empty())
    } {
        assert!(Instant::now() < deadline, "the reconciler never went idle");
        std::thread::sleep(Duration::from_secs(1));
    }

    let (ok, text) = run(&["snapshot", leader]);
    assert!(ok && text.contains("consensus: state=leader"), "{text}");
    let (ok, text) = run(&["status", leader]);
    assert!(ok && text.contains("  - worker1@worker1:4370"), "{text}");
    let (ok, text) = run(&["routing", leader]);
    assert!(ok && text.contains("peer: node="), "{text}");
    let (ok, text) = run(&["scaling", leader]);
    assert!(ok && text.contains("continuity_store:"), "{text}");

    let control = |args: &[&str]| {
        let mut all = args.to_vec();
        all.extend(["--cluster-id", cluster_id, "--operator-key-file", key_file]);
        let (ok, text) = run(&all);
        assert!(ok, "{args:?}: {text}");
        text
    };
    let text = control(&["autoscale", "pause", leader]);
    assert!(
        text.contains("accepted: true") && text.contains("autoscaler_paused: true"),
        "{text}"
    );
    let outcome: Value = serde_json::from_str(&control(&["scale", leader, "3", "--json"])).unwrap();
    assert_eq!(outcome["accepted"], true, "{outcome}");
    assert_eq!(outcome["desired_capacity_override"], 3, "{outcome}");
    // The drains the reconciler finished are over: only this one shows.
    let text = control(&["drain", leader, "worker2@worker2:4370"]);
    assert!(
        text.contains("drain_intents: worker2@worker2:4370\n"),
        "{text}"
    );
    let text = control(&["cancel-drain", leader, "worker2@worker2:4370"]);
    assert!(text.contains("drain_intents: \n"), "{text}");
    let text = control(&["autoscale", "resume", leader]);
    assert!(text.contains("autoscaler_paused: false"), "{text}");
}
