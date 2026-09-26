use std::env;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const UPDATE_INSTALLER_URL_ENV: &str = "MESH_UPDATE_INSTALLER_URL";
const DEFAULT_UNIX_INSTALLER_URL: &str = "https://meshlang.dev/install.sh";
const DEFAULT_WINDOWS_INSTALLER_URL: &str = "https://meshlang.dev/install.ps1";
const DEFAULT_DOWNLOAD_TIMEOUT_SEC: u64 = 120;
const WINDOWS_BOOTSTRAP_SETTLE_MS: u64 = 50;
const WAIT_FAILED: &str = "failed while waiting for installer process";
const FORWARDED_INSTALLER_ENV_KEYS: [&str; 4] = [
    "MESH_INSTALL_RELEASE_API_URL",
    "MESH_INSTALL_RELEASE_BASE_URL",
    "MESH_INSTALL_DOWNLOAD_TIMEOUT_SEC",
    "MESH_INSTALL_STRICT_PROOF",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolchainUpdateMode {
    Completed,
    DetachedBootstrap,
}

/// What an update that got this far tells its user.
impl fmt::Display for ToolchainUpdateMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Completed => "Mesh toolchain update completed via the canonical installer.",
            Self::DetachedBootstrap => "Mesh toolchain update bootstrap launched; the installer will finish replacing the toolchain after this process exits.",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainUpdateOutcome {
    pub installer_url: String,
    pub mode: ToolchainUpdateMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainUpdateError {
    phase: &'static str,
    platform: String,
    installer_url: String,
    launcher: Option<String>,
    detail: String,
}

impl fmt::Display for ToolchainUpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.launcher {
            Some(launcher) => write!(
                f,
                "toolchain update {} failed on {} with launcher {} for installer {}: {}",
                self.phase, self.platform, launcher, self.installer_url, self.detail
            ),
            None => write!(
                f,
                "toolchain update {} failed on {} for installer {}: {}",
                self.phase, self.platform, self.installer_url, self.detail
            ),
        }
    }
}

impl std::error::Error for ToolchainUpdateError {}

/// Where an update attempt is, for the errors it reports.
#[derive(Clone, Copy)]
struct Attempt<'a> {
    platform: &'a str,
    installer_url: &'a str,
    launcher: Option<&'a str>,
}

impl<'a> Attempt<'a> {
    fn new(platform: &'a ToolchainUpdatePlatform, installer_url: &'a str) -> Self {
        Self {
            platform: platform.label(),
            installer_url,
            launcher: None,
        }
    }

    fn with_launcher(self, launcher: &'a str) -> Self {
        Self {
            launcher: Some(launcher),
            ..self
        }
    }

    fn error(&self, phase: &'static str, detail: impl Into<String>) -> ToolchainUpdateError {
        ToolchainUpdateError {
            phase,
            platform: self.platform.to_string(),
            installer_url: self.installer_url.to_string(),
            launcher: self.launcher.map(str::to_string),
            detail: detail.into(),
        }
    }

    /// The error for a step that failed: what it was doing, then why.
    fn failed<E: fmt::Display>(
        self,
        phase: &'static str,
        doing: impl Into<String>,
    ) -> impl Fn(E) -> ToolchainUpdateError + 'a {
        let doing = doing.into();
        move |cause| self.error(phase, format!("{doing}: {cause}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolchainUpdatePlatform {
    Unix,
    Windows,
    Unsupported(String),
}

impl ToolchainUpdatePlatform {
    pub(crate) fn detect() -> Self {
        Self::of(env::consts::FAMILY, env::consts::OS)
    }

    /// The platform of an OS `family` (`std::env::consts::FAMILY`) and name.
    fn of(family: &str, os: &str) -> Self {
        match family {
            "windows" => Self::Windows,
            "unix" => Self::Unix,
            _ => Self::Unsupported(os.to_string()),
        }
    }

    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Unix => "unix",
            Self::Windows => "windows",
            Self::Unsupported(platform) => platform.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ToolchainUpdateEnv {
    installer_url_override: Option<String>,
    download_timeout_sec: Option<String>,
    forwarded_env: Vec<(String, String)>,
}

impl ToolchainUpdateEnv {
    pub(crate) fn capture() -> Self {
        Self::from_lookup(|key| env::var(key).ok())
    }

    pub(crate) fn from_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let installer_url_override = lookup(UPDATE_INSTALLER_URL_ENV);
        let download_timeout_sec = lookup("MESH_INSTALL_DOWNLOAD_TIMEOUT_SEC");
        let forwarded_env = FORWARDED_INSTALLER_ENV_KEYS
            .iter()
            .filter_map(|key| lookup(key).map(|value| ((*key).to_string(), value)))
            .collect();

        Self {
            installer_url_override,
            download_timeout_sec,
            forwarded_env,
        }
    }

    pub(crate) fn installer_url_for(
        &self,
        platform: &ToolchainUpdatePlatform,
    ) -> Result<String, ToolchainUpdateError> {
        match &self.installer_url_override {
            Some(url) => Ok(url.clone()),
            None => default_installer_url(platform).map(str::to_owned),
        }
    }

    pub(crate) fn download_timeout(&self) -> Duration {
        let parsed = self
            .download_timeout_sec
            .as_deref()
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_DOWNLOAD_TIMEOUT_SEC);
        Duration::from_secs(parsed)
    }

    pub(crate) fn forwarded_env(&self) -> &[(String, String)] {
        &self.forwarded_env
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LauncherCommand {
    pub program: String,
    pub args: Vec<String>,
}

pub fn run_toolchain_update() -> Result<ToolchainUpdateOutcome, ToolchainUpdateError> {
    run_toolchain_update_on(
        &ToolchainUpdatePlatform::detect(),
        &ToolchainUpdateEnv::capture(),
        &env::temp_dir(),
    )
}

/// The update on `platform`, as `update_env` sets it up; a Windows
/// bootstrap writes its scripts under `temp_root`.
pub(crate) fn run_toolchain_update_on(
    platform: &ToolchainUpdatePlatform,
    update_env: &ToolchainUpdateEnv,
    temp_root: &Path,
) -> Result<ToolchainUpdateOutcome, ToolchainUpdateError> {
    let installer_url = update_env.installer_url_for(platform)?;
    let installer_text =
        download_installer_script(&installer_url, update_env.download_timeout(), platform)?;

    match platform {
        ToolchainUpdatePlatform::Unix => run_unix_installer_with_command(
            &installer_text,
            update_env.forwarded_env(),
            &installer_url,
            &unix_launcher_command(),
        ),
        ToolchainUpdatePlatform::Windows => launch_windows_bootstrap(
            &installer_text,
            update_env.forwarded_env(),
            &installer_url,
            std::process::id(),
            temp_root,
        ),
        ToolchainUpdatePlatform::Unsupported(_) => Err(Attempt::new(platform, &installer_url)
            .error("plan-launcher", "unsupported host platform")),
    }
}

pub(crate) fn default_installer_url(
    platform: &ToolchainUpdatePlatform,
) -> Result<&'static str, ToolchainUpdateError> {
    match platform {
        ToolchainUpdatePlatform::Unix => Ok(DEFAULT_UNIX_INSTALLER_URL),
        ToolchainUpdatePlatform::Windows => Ok(DEFAULT_WINDOWS_INSTALLER_URL),
        ToolchainUpdatePlatform::Unsupported(_) => {
            Err(Attempt::new(platform, "<unsupported-platform>")
                .error("plan-launcher", "unsupported host platform"))
        }
    }
}

pub(crate) fn download_installer_script(
    installer_url: &str,
    timeout: Duration,
    platform: &ToolchainUpdatePlatform,
) -> Result<String, ToolchainUpdateError> {
    let attempt = Attempt::new(platform, installer_url);
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = agent.get(installer_url).call().map_err(attempt.failed(
        "download",
        format!(
            "failed to fetch installer with timeout {}s",
            timeout.as_secs()
        ),
    ))?;

    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut bytes)
        .map_err(attempt.failed("download", "failed to read installer response body"))?;

    validate_installer_bytes(&bytes, installer_url, platform)
}

pub(crate) fn validate_installer_bytes(
    bytes: &[u8],
    installer_url: &str,
    platform: &ToolchainUpdatePlatform,
) -> Result<String, ToolchainUpdateError> {
    let attempt = Attempt::new(platform, installer_url);
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| {
        attempt.error(
            "download",
            "installer response body was not valid UTF-8 text",
        )
    })?;
    if text.trim().is_empty() {
        return Err(attempt.error("download", "installer response body was empty"));
    }
    Ok(text)
}

pub(crate) fn unix_launcher_command() -> LauncherCommand {
    LauncherCommand {
        program: "/bin/sh".to_string(),
        args: vec!["-s".to_string(), "--".to_string(), "--yes".to_string()],
    }
}

pub(crate) fn run_unix_installer_with_command(
    installer_text: &str,
    forwarded_env: &[(String, String)],
    installer_url: &str,
    launcher: &LauncherCommand,
) -> Result<ToolchainUpdateOutcome, ToolchainUpdateError> {
    let attempt = Attempt::new(&ToolchainUpdatePlatform::Unix, installer_url)
        .with_launcher(&launcher.program);
    let mut child = Command::new(&launcher.program)
        .args(&launcher.args)
        .envs(forwarded_env.iter().cloned())
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(attempt.failed("spawn-launcher", "failed to spawn launcher"))?;

    // Dropped once written, so the launcher sees the script end.
    let written = child
        .stdin
        .take()
        .expect("the launcher's stdin is piped")
        .write_all(installer_text.as_bytes());
    let status = child
        .wait()
        .map_err(attempt.failed("wait-launcher", WAIT_FAILED))?;
    if !status.success() {
        return Err(attempt.error(
            "wait-launcher",
            format!("installer exited with status {}", status),
        ));
    }
    // A launcher that succeeded without reading all of the script ran
    // something else.
    written.map_err(attempt.failed(
        "wait-launcher",
        "failed to write installer to launcher stdin",
    ))?;

    Ok(ToolchainUpdateOutcome {
        installer_url: installer_url.to_string(),
        mode: ToolchainUpdateMode::Completed,
    })
}

pub(crate) fn windows_launcher_command(bootstrap_path: &Path) -> LauncherCommand {
    LauncherCommand {
        program: "powershell.exe".to_string(),
        args: vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            bootstrap_path.to_string_lossy().into_owned(),
        ],
    }
}

pub(crate) fn build_windows_bootstrap_script(installer_path: &Path, parent_pid: u32) -> String {
    let installer_literal = powershell_single_quote(installer_path);
    format!(
        "$ErrorActionPreference = 'Stop'\n\
$ParentPid = {parent_pid}\n\
$InstallerPath = '{installer_literal}'\n\
if (-not (Test-Path $InstallerPath)) {{\n\
    Write-Error \"installer script missing at $InstallerPath\"\n\
    exit 1\n\
}}\n\
$Attempts = 0\n\
while ((Get-Process -Id $ParentPid -ErrorAction SilentlyContinue) -and ($Attempts -lt 100)) {{\n\
    Start-Sleep -Milliseconds 200\n\
    $Attempts += 1\n\
}}\n\
try {{\n\
    & $InstallerPath -Yes\n\
    $ExitCode = $LASTEXITCODE\n\
}} catch {{\n\
    Write-Error $_\n\
    exit 1\n\
}}\n\
if ($null -eq $ExitCode) {{\n\
    $ExitCode = 0\n\
}}\n\
exit $ExitCode\n"
    )
}

pub(crate) fn write_script_file(
    path: &Path,
    contents: &str,
    installer_url: &str,
    platform: &ToolchainUpdatePlatform,
) -> Result<(), ToolchainUpdateError> {
    let path_label = path.display().to_string();
    fs::write(path, contents).map_err(
        Attempt::new(platform, installer_url)
            .with_launcher(&path_label)
            .failed(
                "write-installer",
                format!("failed to write script file {}", path.display()),
            ),
    )
}

pub(crate) fn spawn_windows_bootstrap_command(
    launcher: &LauncherCommand,
    forwarded_env: &[(String, String)],
    installer_url: &str,
) -> Result<ToolchainUpdateOutcome, ToolchainUpdateError> {
    let attempt = Attempt::new(&ToolchainUpdatePlatform::Windows, installer_url)
        .with_launcher(&launcher.program);
    let mut child = Command::new(&launcher.program)
        .args(&launcher.args)
        .envs(forwarded_env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(attempt.failed("spawn-launcher", "failed to spawn launcher"))?;

    thread::sleep(Duration::from_millis(WINDOWS_BOOTSTRAP_SETTLE_MS));
    match child
        .try_wait()
        .map_err(attempt.failed("bootstrap", "failed to inspect bootstrap status"))?
    {
        Some(status) if !status.success() => Err(attempt.error(
            "bootstrap",
            format!("bootstrap exited early with status {}", status),
        )),
        _ => Ok(ToolchainUpdateOutcome {
            installer_url: installer_url.to_string(),
            mode: ToolchainUpdateMode::DetachedBootstrap,
        }),
    }
}

/// Write the installer and a bootstrap that runs it once this process has
/// exited (Windows cannot replace a running executable) into a new
/// directory under `temp_root`, and start the bootstrap.
pub(crate) fn launch_windows_bootstrap(
    installer_text: &str,
    forwarded_env: &[(String, String)],
    installer_url: &str,
    parent_pid: u32,
    temp_root: &Path,
) -> Result<ToolchainUpdateOutcome, ToolchainUpdateError> {
    let platform = ToolchainUpdatePlatform::Windows;
    let temp_dir = temp_root.join(unique_temp_dir_name());
    let temp_label = temp_dir.display().to_string();
    fs::create_dir_all(&temp_dir).map_err(
        Attempt::new(&platform, installer_url)
            .with_launcher(&temp_label)
            .failed(
                "write-installer",
                format!(
                    "failed to create temp script directory {}",
                    temp_dir.display()
                ),
            ),
    )?;
    let installer_path = temp_dir.join("install.ps1");
    let bootstrap_path = temp_dir.join("mesh-update-bootstrap.ps1");
    let bootstrap = build_windows_bootstrap_script(&installer_path, parent_pid);
    for (path, contents) in [
        (&installer_path, installer_text),
        (&bootstrap_path, &bootstrap),
    ] {
        write_script_file(path, contents, installer_url, &platform)?;
    }
    spawn_windows_bootstrap_command(
        &windows_launcher_command(&bootstrap_path),
        forwarded_env,
        installer_url,
    )
}

fn unique_temp_dir_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("mesh-toolchain-update-{}-{}", std::process::id(), nanos)
}

fn powershell_single_quote(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::net::TcpListener;
    use tempfile::TempDir;

    fn env_from_pairs(pairs: &[(&str, &str)]) -> ToolchainUpdateEnv {
        let values: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        ToolchainUpdateEnv::from_lookup(|key| values.get(key).cloned())
    }

    /// Serve one response, `body` declared as `declared_length` bytes.
    fn serve_once_declaring(body: &[u8], declared_length: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have an address");
        let body = body.to_vec();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("server should accept");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {declared_length}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(headers.as_bytes())
                .expect("headers should write");
            stream.write_all(&body).expect("body should write");
        });
        format!("http://{addr}/install.sh")
    }

    fn serve_once(body: &[u8]) -> String {
        serve_once_declaring(body, body.len())
    }

    fn sh(script: &str) -> LauncherCommand {
        LauncherCommand {
            program: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
        }
    }

    #[test]
    fn installer_urls_are_the_public_ones_unless_overridden() {
        assert_eq!(
            default_installer_url(&ToolchainUpdatePlatform::Unix).unwrap(),
            "https://meshlang.dev/install.sh"
        );
        assert_eq!(
            default_installer_url(&ToolchainUpdatePlatform::Windows).unwrap(),
            "https://meshlang.dev/install.ps1"
        );
        let error = default_installer_url(&ToolchainUpdatePlatform::Unsupported("plan9".into()))
            .expect_err("unsupported platforms should fail closed");
        assert_eq!(
            (error.phase, error.platform.as_str()),
            ("plan-launcher", "plan9")
        );
        assert!(
            error.to_string().contains("unsupported host platform"),
            "{error}"
        );

        let env = env_from_pairs(&[(
            "MESH_UPDATE_INSTALLER_URL",
            "http://127.0.0.1:9000/custom.ps1",
        )]);
        for platform in [
            ToolchainUpdatePlatform::Unix,
            ToolchainUpdatePlatform::Windows,
        ] {
            assert_eq!(
                env.installer_url_for(&platform).unwrap(),
                "http://127.0.0.1:9000/custom.ps1"
            );
        }
        assert_eq!(
            env_from_pairs(&[])
                .installer_url_for(&ToolchainUpdatePlatform::Unix)
                .unwrap(),
            "https://meshlang.dev/install.sh"
        );
        #[cfg(unix)]
        assert_eq!(
            ToolchainUpdatePlatform::detect(),
            ToolchainUpdatePlatform::Unix
        );
    }

    #[test]
    fn forwards_only_supported_mesh_install_overrides_in_fixed_order() {
        let env = env_from_pairs(&[
            (
                "MESH_INSTALL_RELEASE_API_URL",
                "http://127.0.0.1:9000/api/releases/latest.json",
            ),
            (
                "MESH_INSTALL_RELEASE_BASE_URL",
                "http://127.0.0.1:9000/download",
            ),
            ("MESH_INSTALL_DOWNLOAD_TIMEOUT_SEC", "20"),
            ("MESH_INSTALL_STRICT_PROOF", "1"),
            ("UNRELATED_ENV", "ignored"),
        ]);
        let keys: Vec<&str> = env
            .forwarded_env()
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        assert_eq!(keys, FORWARDED_INSTALLER_ENV_KEYS);
        assert_eq!(env.download_timeout(), Duration::from_secs(20));
        for timeout in ["0", "soon"] {
            assert_eq!(
                env_from_pairs(&[("MESH_INSTALL_DOWNLOAD_TIMEOUT_SEC", timeout)])
                    .download_timeout(),
                Duration::from_secs(DEFAULT_DOWNLOAD_TIMEOUT_SEC)
            );
        }
    }

    #[test]
    fn launchers_run_the_installer_as_the_platform_does() {
        let launcher = unix_launcher_command();
        assert_eq!(launcher.program, "/bin/sh");
        assert_eq!(launcher.args, ["-s", "--", "--yes"]);

        let launcher = windows_launcher_command(Path::new(r"C:\Temp\mesh-update-bootstrap.ps1"));
        assert_eq!(launcher.program, "powershell.exe");
        assert_eq!(
            launcher.args,
            [
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                r"C:\Temp\mesh-update-bootstrap.ps1"
            ]
        );

        let script = build_windows_bootstrap_script(Path::new(r"C:\Temp\it's\install.ps1"), 4242);
        assert!(script.contains("$ParentPid = 4242"), "{script}");
        assert!(
            script.contains(r"$InstallerPath = 'C:\Temp\it''s\install.ps1'"),
            "{script}"
        );
        assert!(
            script.contains("while ((Get-Process -Id $ParentPid -ErrorAction SilentlyContinue) -and ($Attempts -lt 100))"),
            "{script}"
        );
        assert!(script.contains("& $InstallerPath -Yes"), "{script}");
    }

    #[test]
    fn downloads_are_refused_for_what_is_wrong_with_them() {
        let unix = ToolchainUpdatePlatform::Unix;
        let error = download_installer_script("not a url", Duration::from_secs(1), &unix)
            .expect_err("malformed URLs should fail before execution");
        assert_eq!(
            (error.phase, error.installer_url.as_str()),
            ("download", "not a url")
        );

        for (body, declared, expected) in [
            (&b""[..], 0, "was empty"),
            (&b" \n\t"[..], 3, "was empty"),
            (&[0xff, 0xfe, 0xfd][..], 3, "valid UTF-8"),
            (
                &b"exit 0\n"[..],
                100,
                "failed to read installer response body",
            ),
        ] {
            let url = serve_once_declaring(body, declared);
            let error =
                download_installer_script(&url, Duration::from_secs(5), &unix).expect_err(expected);
            assert_eq!(
                (error.phase, error.installer_url.as_str()),
                ("download", url.as_str())
            );
            assert!(error.to_string().contains(expected), "{expected}: {error}");
        }
        let url = serve_once(b"exit 0\n");
        assert_eq!(
            download_installer_script(&url, Duration::from_secs(5), &unix).unwrap(),
            "exit 0\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_unix_installer_runs_to_its_end() {
        let url = "https://meshlang.dev/install.sh";
        let outcome =
            run_unix_installer_with_command("exit 0\n", &[], url, &unix_launcher_command())
                .unwrap();
        assert_eq!(outcome.mode, ToolchainUpdateMode::Completed);

        let error = run_unix_installer_with_command("exit 3\n", &[], url, &unix_launcher_command())
            .expect_err("a failed installer is an error");
        assert_eq!(error.phase, "wait-launcher");
        assert!(
            error.to_string().contains("installer exited with status"),
            "{error}"
        );

        // A launcher that ends without reading the script did not run it.
        let script = "#".repeat(1 << 20);
        let error = run_unix_installer_with_command(&script, &[], url, &sh("exit 0"))
            .expect_err("an unread installer is an error");
        assert!(
            error.to_string().contains("failed to write installer"),
            "{error}"
        );

        let missing = LauncherCommand {
            program: "__missing_mesh_unix_launcher__".to_string(),
            args: vec!["-s".to_string()],
        };
        let error = run_unix_installer_with_command("exit 0\n", &[], url, &missing)
            .expect_err("missing launchers should fail closed");
        assert_eq!(error.phase, "spawn-launcher");
        assert_eq!(
            error.launcher.as_deref(),
            Some("__missing_mesh_unix_launcher__")
        );
    }

    /// The whole update: download from the override URL, then run it.
    #[cfg(unix)]
    #[test]
    fn an_update_downloads_and_runs_the_installer() {
        let url = serve_once(b"exit 0\n");
        // Only this test reads the variable.
        env::set_var(UPDATE_INSTALLER_URL_ENV, &url);
        let outcome = run_toolchain_update();
        env::remove_var(UPDATE_INSTALLER_URL_ENV);
        assert_eq!(
            outcome.unwrap(),
            ToolchainUpdateOutcome {
                installer_url: url,
                mode: ToolchainUpdateMode::Completed
            }
        );
    }

    #[test]
    fn each_outcome_says_where_the_update_is() {
        assert!(ToolchainUpdateMode::Completed
            .to_string()
            .contains("completed"));
        assert!(ToolchainUpdateMode::DetachedBootstrap
            .to_string()
            .contains("after this process exits"));
    }

    /// Each platform runs the installer its way; one with no way is refused
    /// once the installer is in hand.
    #[cfg(unix)]
    #[test]
    fn updates_run_by_platform() {
        assert_eq!(
            ToolchainUpdatePlatform::of("windows", "windows"),
            ToolchainUpdatePlatform::Windows
        );
        assert_eq!(
            ToolchainUpdatePlatform::of("unix", "macos"),
            ToolchainUpdatePlatform::Unix
        );
        assert_eq!(
            ToolchainUpdatePlatform::of("", "uefi"),
            ToolchainUpdatePlatform::Unsupported("uefi".to_string())
        );

        let temp = TempDir::new().unwrap();
        let update = |platform: ToolchainUpdatePlatform| {
            let update_env =
                env_from_pairs(&[(UPDATE_INSTALLER_URL_ENV, &serve_once(b"exit 0\n"))]);
            run_toolchain_update_on(&platform, &update_env, temp.path()).unwrap_err()
        };
        let error = update(ToolchainUpdatePlatform::Unsupported("uefi".to_string()));
        assert_eq!(error.phase, "plan-launcher");
        // No powershell.exe here: the bootstrap is written, and starting it fails.
        let error = update(ToolchainUpdatePlatform::Windows);
        assert!(error.detail.contains("failed to spawn launcher"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn the_windows_bootstrap_is_written_then_started() {
        let url = "https://meshlang.dev/install.ps1";
        let temp = TempDir::new().unwrap();
        // No powershell.exe here: the scripts are written, and starting fails.
        let error = launch_windows_bootstrap("Write-Host hi", &[], url, 4242, temp.path())
            .expect_err("powershell.exe is not on this host");
        assert_eq!(error.phase, "spawn-launcher");
        let dir = fs::read_dir(temp.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::read_to_string(dir.join("install.ps1")).unwrap(),
            "Write-Host hi"
        );
        assert!(fs::read_to_string(dir.join("mesh-update-bootstrap.ps1"))
            .unwrap()
            .contains("$ParentPid = 4242"));

        let not_a_directory = temp.path().join("file");
        fs::write(&not_a_directory, "").unwrap();
        let error = launch_windows_bootstrap("", &[], url, 1, &not_a_directory)
            .expect_err("no directory can be made under a file");
        assert_eq!(error.phase, "write-installer");
        let error = write_script_file(temp.path(), "", url, &ToolchainUpdatePlatform::Windows)
            .expect_err("writing to a directory should fail");
        assert_eq!(error.phase, "write-installer");
        assert!(error
            .to_string()
            .contains(temp.path().to_string_lossy().as_ref()));

        let outcome = spawn_windows_bootstrap_command(&sh("exit 0"), &[], url).unwrap();
        assert_eq!(outcome.mode, ToolchainUpdateMode::DetachedBootstrap);
        // A bootstrap that fails before it detaches is reported.
        let error = spawn_windows_bootstrap_command(&sh("exit 3"), &[], url).unwrap_err();
        assert_eq!(error.phase, "bootstrap");
        assert!(
            error.to_string().contains("bootstrap exited early"),
            "{error}"
        );
    }
}
