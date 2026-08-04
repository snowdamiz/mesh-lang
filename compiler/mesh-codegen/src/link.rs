//! Object file linking via a target-aware system linker driver.
//!
//! Links compiled object files with the Mesh runtime static library to produce
//! native executables. Unix targets keep using the system C compiler driver,
//! while Windows MSVC targets use `clang`/`clang.exe` so the installed
//! compiler does not assume Unix tool names or library naming.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::build_trace;

/// Link an object file with the Mesh runtime to produce a native executable.
///
/// # Arguments
///
/// * `object_path` - Path to the compiled object file
/// * `output_path` - Path for the output executable
/// * `target_triple` - Optional target triple for linker/runtime selection
/// * `rt_lib_path` - Optional path to the Mesh runtime static library; if None,
///   attempts to locate it in the workspace target directory
///
/// # Errors
///
/// Returns an error string if the linker cannot be found or linking fails.
pub fn link(
    object_path: &Path,
    output_path: &Path,
    target_triple: Option<&str>,
    rt_lib_path: Option<&Path>,
) -> Result<(), String> {
    let plan = prepare_link(target_triple, rt_lib_path)?;
    link_with_plan(object_path, output_path, &plan)
}

pub fn effective_target_triple(target_triple: Option<&str>) -> Result<String, String> {
    LinkTarget::detect(target_triple).map(|target| target.display_triple())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFlavor {
    Standard,
    Test,
}

impl RuntimeFlavor {
    fn package_name(self) -> &'static str {
        match self {
            Self::Standard => "mesh-rt",
            Self::Test => "mesh-test-rt",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Standard => "Mesh runtime",
            Self::Test => "Mesh test runtime",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkPlan {
    target: LinkTarget,
    rt_path: PathBuf,
    linker_program: PathBuf,
    native_archives: Vec<PathBuf>,
}

pub(crate) fn prepare_link(
    target_triple: Option<&str>,
    rt_lib_path: Option<&Path>,
) -> Result<LinkPlan, String> {
    prepare_link_for_runtime(target_triple, rt_lib_path, &[], RuntimeFlavor::Standard)
}

pub(crate) fn prepare_link_with_native(
    target_triple: Option<&str>,
    rt_lib_path: Option<&Path>,
    native_archives: &[PathBuf],
) -> Result<LinkPlan, String> {
    prepare_link_for_runtime(
        target_triple,
        rt_lib_path,
        native_archives,
        RuntimeFlavor::Standard,
    )
}

pub(crate) fn prepare_test_link_with_native(
    target_triple: Option<&str>,
    rt_lib_path: Option<&Path>,
    native_archives: &[PathBuf],
) -> Result<LinkPlan, String> {
    prepare_link_for_runtime(
        target_triple,
        rt_lib_path,
        native_archives,
        RuntimeFlavor::Test,
    )
}

fn prepare_link_for_runtime(
    target_triple: Option<&str>,
    rt_lib_path: Option<&Path>,
    native_archives: &[PathBuf],
    runtime_flavor: RuntimeFlavor,
) -> Result<LinkPlan, String> {
    let target = LinkTarget::detect(target_triple)?;
    build_trace::set_stage("resolve-runtime-library");

    let rt_path = match rt_lib_path {
        Some(path) => match validate_runtime_override(path, &target, runtime_flavor) {
            Ok(()) => path.to_path_buf(),
            Err(error) => {
                build_trace::set_link_context(
                    &target.display_triple(),
                    Some(path),
                    Some(path.exists()),
                    None,
                );
                build_trace::record_error(&error);
                return Err(error);
            }
        },
        None => match find_mesh_rt(&target, runtime_flavor) {
            Ok(path) => path,
            Err(error) => {
                build_trace::set_link_context(&target.display_triple(), None, Some(false), None);
                build_trace::record_error(&error);
                return Err(error);
            }
        },
    };

    let runtime_exists = rt_path.exists();
    let linker_program = match target.linker_program() {
        Ok(path) => path,
        Err(error) => {
            build_trace::set_link_context(
                &target.display_triple(),
                Some(&rt_path),
                Some(runtime_exists),
                None,
            );
            build_trace::record_error(&error);
            return Err(error);
        }
    };
    build_trace::set_link_context(
        &target.display_triple(),
        Some(&rt_path),
        Some(runtime_exists),
        Some(&linker_program),
    );

    if !runtime_exists {
        let error = format!(
            "{} static library not found at '{}'. Expected {} for target '{}'. Run `cargo build -p {}{}` first.",
            runtime_flavor.display_name(),
            rt_path.display(),
            target.runtime_filename(runtime_flavor),
            target.display_triple(),
            runtime_flavor.package_name(),
            target.cargo_build_hint(),
        );
        build_trace::record_error(&error);
        return Err(error);
    }

    for archive in native_archives {
        validate_native_archive(archive, &target)?;
    }

    Ok(LinkPlan {
        target,
        rt_path,
        linker_program,
        native_archives: native_archives.to_vec(),
    })
}

pub(crate) fn link_with_plan(
    object_path: &Path,
    output_path: &Path,
    plan: &LinkPlan,
) -> Result<(), String> {
    let mut cmd = build_link_command(object_path, output_path, plan);

    build_trace::mark_link_started();
    let output = match cmd.output() {
        Ok(output) => output,
        Err(error) => {
            let error = format!(
                "Failed to invoke linker '{}': {}.{}",
                plan.linker_program.display(),
                error,
                plan.target.linker_help_suffix(),
            );
            build_trace::record_error(&error);
            return Err(error);
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() {
            format!("stderr:\n{stderr}")
        } else if !stdout.is_empty() {
            format!("stdout:\n{stdout}")
        } else {
            format!(
                "linker exited with status {} without emitting output",
                output.status
            )
        };

        let error = format!(
            "Linking failed for target '{}'.\nlinker: {}\nruntime: {}\n{}",
            plan.target.display_triple(),
            plan.linker_program.display(),
            plan.rt_path.display(),
            detail,
        );
        build_trace::record_error(&error);
        return Err(error);
    }

    build_trace::mark_link_completed();
    std::fs::remove_file(object_path).ok();
    Ok(())
}

pub(crate) fn archive_with_plan(
    object_path: &Path,
    output_path: &Path,
    plan: &LinkPlan,
) -> Result<(), String> {
    let output = if plan.target.is_apple() {
        let mut command = Command::new("xcrun");
        command.args(["libtool", "-static", "-o"]);
        command.arg(output_path).arg(object_path);
        for archive in &plan.native_archives {
            command.arg(archive);
        }
        command.arg(&plan.rt_path).output()
    } else if plan.target.kind == LinkTargetKind::Unix {
        let archiver = plan.target.archiver_program()?;
        let child = Command::new(&archiver)
            .arg("-M")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        match child {
            Ok(mut child) => {
                let mut script = format!(
                    "CREATE {}\nADDMOD {}\n",
                    output_path.display(),
                    object_path.display()
                );
                for archive in &plan.native_archives {
                    script.push_str(&format!("ADDLIB {}\n", archive.display()));
                }
                script.push_str(&format!("ADDLIB {}\nSAVE\nEND\n", plan.rt_path.display()));
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(script.as_bytes()).map_err(|error| {
                        format!("Failed to drive '{}': {error}", archiver.display())
                    })?;
                }
                child.wait_with_output()
            }
            Err(error) => Err(error),
        }
    } else {
        return Err("static library artifacts are not yet supported for Windows MSVC".to_string());
    }
    .map_err(|error| format!("Failed to create static library: {error}"))?;

    finish_library_link(output, object_path, output_path, "Static library creation")
}

pub(crate) fn link_dynamic_with_plan(
    object_path: &Path,
    output_path: &Path,
    plan: &LinkPlan,
) -> Result<(), String> {
    if plan.target.kind != LinkTargetKind::Unix {
        return Err("dynamic library artifacts are not yet supported for Windows MSVC".to_string());
    }
    let mut command = plan.target.dynamic_linker_command()?;
    command.arg(object_path);
    for archive in &plan.native_archives {
        command.arg(archive);
    }
    if plan.target.is_apple() {
        command
            .arg(format!("-Wl,-force_load,{}", plan.rt_path.display()))
            .arg("-dynamiclib");
    } else {
        command
            .arg("-Wl,--whole-archive")
            .arg(&plan.rt_path)
            .arg("-Wl,--no-whole-archive")
            .arg("-shared");
    }
    command.arg("-lm").arg("-o").arg(output_path);
    if plan.target.needs_security_framework() {
        for framework in ["Security", "CoreFoundation"] {
            command.arg("-framework").arg(framework);
        }
    }
    let output = command
        .output()
        .map_err(|error| format!("Failed to invoke dynamic linker: {error}"))?;
    finish_library_link(output, object_path, output_path, "Dynamic library linking")
}

fn finish_library_link(
    output: std::process::Output,
    object_path: &Path,
    output_path: &Path,
    operation: &str,
) -> Result<(), String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(format!(
            "{operation} failed for '{}': {detail}",
            output_path.display()
        ));
    }
    std::fs::remove_file(object_path).ok();
    Ok(())
}

fn build_link_command(object_path: &Path, output_path: &Path, plan: &LinkPlan) -> Command {
    let mut cmd = Command::new(&plan.linker_program);
    cmd.arg(object_path);
    for archive in &plan.native_archives {
        cmd.arg(archive);
    }

    match plan.target.kind {
        LinkTargetKind::Unix => {
            cmd.arg(&plan.rt_path).arg("-lm").arg("-o").arg(output_path);
        }
        LinkTargetKind::WindowsMsvc => {
            cmd.arg(&plan.rt_path).arg("-o").arg(output_path);
            // mesh_rt.lib is a Rust staticlib whose transitive deps (ureq/TLS,
            // sqlite, crossbeam, rand, Rust std) need these Windows system libraries.
            // Use -Wl, to forward them directly to MSVC's link.exe.
            for lib in &[
                "ws2_32.lib",
                "userenv.lib",
                "advapi32.lib",
                "bcrypt.lib",
                "ntdll.lib",
                "kernel32.lib",
                "msvcrt.lib",
                "synchronization.lib",
            ] {
                cmd.arg(format!("-Wl,{lib}"));
            }
            // Verbose mode so link failures show the full link.exe invocation.
            cmd.arg("-v");
        }
    }

    if plan.target.needs_security_framework() {
        // The runtime's TLS stack uses Security and chrono's local-time
        // support reaches CoreFoundation through iana-time-zone. Rust static
        // libraries do not carry these native framework edges into this final
        // non-Cargo link step, so Mesh must spell them out explicitly.
        for framework in ["Security", "CoreFoundation"] {
            cmd.arg("-framework").arg(framework);
        }
    }

    cmd
}

/// Locate the Mesh runtime static library.
///
/// Searches in the workspace target directory under both `debug` and `release`
/// profiles. Prefers the profile matching the compiler's own build: a release
/// `meshc` links the release runtime, a debug `meshc` links the debug runtime.
fn find_mesh_rt(target: &LinkTarget, runtime_flavor: RuntimeFlavor) -> Result<PathBuf, String> {
    let profiles: &[&str] = if cfg!(debug_assertions) {
        &["debug", "release"]
    } else {
        &["release", "debug"]
    };

    let mut searched_paths = Vec::new();

    for target_dir in [find_workspace_target_dir()].iter().flatten() {
        for candidate in mesh_rt_candidates(target_dir, target, profiles, runtime_flavor) {
            if candidate.exists() {
                return Ok(candidate);
            }
            searched_paths.push(candidate);
        }
    }

    let mut message = format!(
        "Could not locate {} static library for target '{}'. Expected {}. Run `cargo build -p {}{}` first.",
        runtime_flavor.display_name(),
        target.display_triple(),
        target.runtime_filename(runtime_flavor),
        runtime_flavor.package_name(),
        target.cargo_build_hint(),
    );

    if !searched_paths.is_empty() {
        message.push_str("\nSearched:\n");
        for path in searched_paths {
            message.push_str("  - ");
            message.push_str(&path.display().to_string());
            message.push('\n');
        }
        message.pop();
    }

    Err(message)
}

fn mesh_rt_candidates(
    target_dir: &Path,
    target: &LinkTarget,
    profiles: &[&str],
    runtime_flavor: RuntimeFlavor,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(triple) = target.requested_triple.as_deref() {
        for profile in profiles {
            candidates.push(
                target_dir
                    .join(triple)
                    .join(profile)
                    .join(target.runtime_filename(runtime_flavor)),
            );
        }
    }

    for profile in profiles {
        candidates.push(
            target_dir
                .join(profile)
                .join(target.runtime_filename(runtime_flavor)),
        );
    }

    candidates
}

fn validate_runtime_override(
    path: &Path,
    target: &LinkTarget,
    runtime_flavor: RuntimeFlavor,
) -> Result<(), String> {
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return Err(format!(
            "Mesh runtime override '{}' does not name a file. Expected {} for target '{}'.",
            path.display(),
            target.runtime_filename(runtime_flavor),
            target.display_triple(),
        ));
    };

    if file_name != target.runtime_filename(runtime_flavor) {
        return Err(format!(
            "Mesh runtime override '{}' does not match expected filename '{}' for target '{}'.",
            path.display(),
            target.runtime_filename(runtime_flavor),
            target.display_triple(),
        ));
    }

    Ok(())
}

fn validate_native_archive(path: &Path, target: &LinkTarget) -> Result<(), String> {
    if !path.is_absolute() || !path.is_file() {
        return Err(format!(
            "Native archive '{}' must be an existing absolute file path",
            path.display()
        ));
    }
    let extension = path.extension().and_then(|value| value.to_str());
    let expected = match target.kind {
        LinkTargetKind::Unix => "a",
        LinkTargetKind::WindowsMsvc => "lib",
    };
    if extension != Some(expected) {
        return Err(format!(
            "Native archive '{}' must use the `.{expected}` static-library extension for target '{}'",
            path.display(),
            target.display_triple()
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkTargetKind {
    Unix,
    WindowsMsvc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkTarget {
    requested_triple: Option<String>,
    kind: LinkTargetKind,
}

impl LinkTarget {
    fn detect(target_triple: Option<&str>) -> Result<Self, String> {
        let kind = match target_triple {
            Some(triple) => classify_requested_target(triple)?,
            None => classify_host_target()?,
        };

        Ok(Self {
            requested_triple: target_triple.map(ToOwned::to_owned),
            kind,
        })
    }

    fn display_triple(&self) -> String {
        self.requested_triple
            .clone()
            .unwrap_or_else(host_target_triple)
    }

    fn runtime_filename(&self, runtime_flavor: RuntimeFlavor) -> &'static str {
        match (self.kind, runtime_flavor) {
            (LinkTargetKind::Unix, RuntimeFlavor::Standard) => "libmesh_rt.a",
            (LinkTargetKind::Unix, RuntimeFlavor::Test) => "libmesh_test_rt.a",
            (LinkTargetKind::WindowsMsvc, RuntimeFlavor::Standard) => "mesh_rt.lib",
            (LinkTargetKind::WindowsMsvc, RuntimeFlavor::Test) => "mesh_test_rt.lib",
        }
    }

    fn linker_program(&self) -> Result<PathBuf, String> {
        match self.kind {
            LinkTargetKind::Unix => Ok(PathBuf::from("cc")),
            LinkTargetKind::WindowsMsvc => windows_clang_path(),
        }
    }

    fn cargo_build_hint(&self) -> String {
        self.requested_triple
            .as_deref()
            .map(|triple| format!(" --target {triple}"))
            .unwrap_or_default()
    }

    fn linker_help_suffix(&self) -> &'static str {
        match self.kind {
            LinkTargetKind::Unix => "",
            LinkTargetKind::WindowsMsvc => {
                " Set LLVM_SYS_211_PREFIX to an LLVM install containing clang.exe or ensure clang.exe is on PATH."
            }
        }
    }

    fn needs_security_framework(&self) -> bool {
        self.requested_triple
            .as_deref()
            .map(|triple| triple.contains("apple-darwin") || triple.contains("apple-ios"))
            .unwrap_or(cfg!(target_os = "macos"))
    }

    fn is_apple(&self) -> bool {
        self.requested_triple
            .as_deref()
            .map(|triple| triple.contains("apple"))
            .unwrap_or(cfg!(target_os = "macos"))
    }

    fn archiver_program(&self) -> Result<PathBuf, String> {
        let Some(triple) = self.requested_triple.as_deref() else {
            return Ok(PathBuf::from("ar"));
        };
        if triple.contains("linux-android") {
            return android_tool("llvm-ar");
        }
        Ok(PathBuf::from("ar"))
    }

    fn dynamic_linker_command(&self) -> Result<Command, String> {
        let Some(triple) = self.requested_triple.as_deref() else {
            return Ok(Command::new("cc"));
        };
        if triple.contains("apple-ios") {
            let sdk = if triple.ends_with("-sim") {
                "iphonesimulator"
            } else {
                "iphoneos"
            };
            let mut command = Command::new("xcrun");
            command.args(["--sdk", sdk, "clang", "-target", triple]);
            return Ok(command);
        }
        if triple.contains("apple-darwin") {
            let mut command = Command::new("xcrun");
            command.args(["--sdk", "macosx", "clang", "-target", triple]);
            return Ok(command);
        }
        if triple.contains("linux-android") {
            let clang_name = format!("{triple}26-clang");
            return android_tool(&clang_name).map(Command::new);
        }
        let mut command = Command::new("cc");
        command.args(["-target", triple]);
        Ok(command)
    }
}

fn android_tool(name: &str) -> Result<PathBuf, String> {
    let ndk = std::env::var_os("ANDROID_NDK_HOME")
        .or_else(|| std::env::var_os("ANDROID_NDK_ROOT"))
        .map(PathBuf::from)
        .ok_or("Android target requires ANDROID_NDK_HOME or ANDROID_NDK_ROOT")?;
    let prebuilt = ndk.join("toolchains/llvm/prebuilt");
    let entries = std::fs::read_dir(&prebuilt).map_err(|error| {
        format!(
            "Android NDK toolchain directory '{}' is unavailable: {error}",
            prebuilt.display()
        )
    })?;
    for entry in entries.flatten() {
        let candidate = entry.path().join("bin").join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "Android NDK tool '{name}' was not found under '{}'",
        prebuilt.display()
    ))
}

fn classify_requested_target(target_triple: &str) -> Result<LinkTargetKind, String> {
    if target_triple.contains("windows-msvc") {
        return Ok(LinkTargetKind::WindowsMsvc);
    }

    if target_triple.contains("windows") {
        return Err(format!(
            "Unsupported linker target triple '{target_triple}'. Only Windows MSVC targets are supported on Windows."
        ));
    }

    if is_unix_like_target(target_triple) {
        return Ok(LinkTargetKind::Unix);
    }

    Err(format!(
        "Unsupported linker target triple '{target_triple}'. Supported linker families are Unix-like targets and Windows MSVC targets."
    ))
}

fn classify_host_target() -> Result<LinkTargetKind, String> {
    if cfg!(all(target_os = "windows", target_env = "msvc")) {
        return Ok(LinkTargetKind::WindowsMsvc);
    }

    if cfg!(target_family = "unix") {
        return Ok(LinkTargetKind::Unix);
    }

    Err(format!(
        "Unsupported host linker target '{}'. Supported linker families are Unix-like targets and Windows MSVC targets.",
        host_target_triple()
    ))
}

fn is_unix_like_target(target_triple: &str) -> bool {
    [
        "apple-darwin",
        "apple-ios",
        "unknown-linux",
        "linux-musl",
        "linux-android",
        "freebsd",
        "netbsd",
        "openbsd",
        "dragonfly",
    ]
    .iter()
    .any(|needle| target_triple.contains(needle))
}

fn host_target_triple() -> String {
    let arch = std::env::consts::ARCH;

    if cfg!(all(target_os = "windows", target_env = "msvc")) {
        format!("{arch}-pc-windows-msvc")
    } else if cfg!(target_os = "macos") {
        format!("{arch}-apple-darwin")
    } else if cfg!(target_os = "linux") {
        format!("{arch}-unknown-linux-gnu")
    } else {
        format!("{arch}-unknown-{}", std::env::consts::OS)
    }
}

fn windows_clang_path() -> Result<PathBuf, String> {
    if let Ok(prefix) = std::env::var("LLVM_SYS_211_PREFIX") {
        let candidate = windows_clang_path_from_prefix(Path::new(&prefix));
        if candidate.exists() {
            return Ok(candidate);
        }

        return Err(format!(
            "LLVM_SYS_211_PREFIX='{}' does not contain bin/clang.exe at '{}'. Install LLVM 21 or set LLVM_SYS_211_PREFIX correctly.",
            prefix,
            candidate.display(),
        ));
    }

    Ok(PathBuf::from("clang"))
}

fn windows_clang_path_from_prefix(prefix: &Path) -> PathBuf {
    prefix.join("bin").join("clang.exe")
}

/// Attempt to find the workspace target directory.
///
/// Uses the `CARGO_TARGET_DIR` env var if set, otherwise walks up from the
/// current executable to find a `target/` directory.
fn find_workspace_target_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR") {
        return Some(PathBuf::from(dir));
    }

    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|path| path.to_path_buf());
        while let Some(current) = dir {
            if current.file_name().is_some_and(|name| name == "target") {
                return Some(current);
            }

            let target_dir = current.join("target");
            if target_dir.exists() {
                return Some(target_dir);
            }

            dir = current.parent().map(|path| path.to_path_buf());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn find_workspace_target_dir_should_find_target_dir_during_cargo_test() {
        assert!(
            find_workspace_target_dir().is_some(),
            "Should find workspace target dir during cargo test"
        );
    }

    #[test]
    fn classify_requested_target_should_reject_unknown_windows_flavor() {
        let error = classify_requested_target("x86_64-pc-windows-gnu").unwrap_err();
        assert!(
            error.contains("Only Windows MSVC targets are supported on Windows."),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn mobile_targets_use_unix_library_abi() {
        for triple in [
            "aarch64-apple-ios",
            "aarch64-apple-ios-sim",
            "aarch64-linux-android",
            "x86_64-linux-android",
        ] {
            assert_eq!(
                classify_requested_target(triple),
                Ok(LinkTargetKind::Unix),
                "{triple}"
            );
        }
    }

    #[test]
    fn mesh_rt_candidates_should_use_windows_runtime_name_inside_target_subdir() {
        let temp_target = unique_temp_target_dir("windows-runtime-name");
        let runtime = temp_target
            .join("x86_64-pc-windows-msvc")
            .join("debug")
            .join("mesh_rt.lib");
        fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        fs::write(&runtime, b"fake").unwrap();

        let target = LinkTarget::detect(Some("x86_64-pc-windows-msvc")).unwrap();
        let found = find_mesh_rt_in(
            &[temp_target.clone()],
            &target,
            &["debug", "release"],
            RuntimeFlavor::Standard,
        )
        .unwrap();
        assert_eq!(found, runtime);

        fs::remove_dir_all(temp_target).unwrap();
    }

    #[test]
    fn mesh_rt_candidates_should_keep_unix_runtime_name_in_profile_root() {
        let temp_target = unique_temp_target_dir("unix-runtime-name");
        let runtime = temp_target.join("debug").join("libmesh_rt.a");
        fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        fs::write(&runtime, b"fake").unwrap();

        let target = LinkTarget::detect(Some("x86_64-unknown-linux-gnu")).unwrap();
        let found = find_mesh_rt_in(
            &[temp_target.clone()],
            &target,
            &["debug", "release"],
            RuntimeFlavor::Standard,
        )
        .unwrap();
        assert_eq!(found, runtime);

        fs::remove_dir_all(temp_target).unwrap();
    }

    #[test]
    fn test_runtime_uses_a_distinct_archive_name_and_package_hint() {
        let temp_target = unique_temp_target_dir("test-runtime-name");
        let runtime = temp_target.join("debug").join("libmesh_test_rt.a");
        fs::create_dir_all(runtime.parent().unwrap()).unwrap();
        fs::write(&runtime, b"fake").unwrap();

        let target = LinkTarget::detect(Some("x86_64-unknown-linux-gnu")).unwrap();
        let found = find_mesh_rt_in(
            &[temp_target.clone()],
            &target,
            &["debug", "release"],
            RuntimeFlavor::Test,
        )
        .unwrap();
        assert_eq!(found, runtime);

        fs::remove_file(&runtime).unwrap();
        let error = find_mesh_rt_in(
            &[temp_target.clone()],
            &target,
            &["debug", "release"],
            RuntimeFlavor::Test,
        )
        .unwrap_err();
        assert!(error.contains("libmesh_test_rt.a"), "{error}");
        assert!(error.contains("cargo build -p mesh-test-rt"), "{error}");

        fs::remove_dir_all(temp_target).unwrap();
    }

    #[test]
    fn find_mesh_rt_in_should_report_target_specific_runtime_name_when_missing() {
        let temp_target = unique_temp_target_dir("windows-missing-runtime");
        let target = LinkTarget::detect(Some("x86_64-pc-windows-msvc")).unwrap();

        let error = find_mesh_rt_in(
            &[temp_target.clone()],
            &target,
            &["debug", "release"],
            RuntimeFlavor::Standard,
        )
        .unwrap_err();
        assert!(
            error.contains("mesh_rt.lib"),
            "missing runtime error should name mesh_rt.lib: {error}"
        );
        assert!(
            error.contains("cargo build -p mesh-rt --target x86_64-pc-windows-msvc"),
            "missing runtime error should include target-aware cargo hint: {error}"
        );

        fs::remove_dir_all(temp_target).unwrap();
    }

    #[test]
    fn explicit_runtime_override_should_reject_wrong_filename_for_windows_target() {
        let target = LinkTarget::detect(Some("x86_64-pc-windows-msvc")).unwrap();
        let error = validate_runtime_override(
            Path::new("/tmp/libmesh_rt.a"),
            &target,
            RuntimeFlavor::Standard,
        )
        .unwrap_err();
        assert!(
            error.contains("expected filename 'mesh_rt.lib'"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn link_command_passes_native_archives_as_paths_before_runtime() {
        let plan = LinkPlan {
            target: LinkTarget::detect(Some("aarch64-apple-darwin")).unwrap(),
            rt_path: PathBuf::from("/tmp/libmesh_rt.a"),
            linker_program: PathBuf::from("cc"),
            native_archives: vec![PathBuf::from("/tmp/libnative_math.a")],
        };
        let command = build_link_command(Path::new("/tmp/main.o"), Path::new("/tmp/app"), &plan);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let native_index = args
            .iter()
            .position(|arg| arg == "/tmp/libnative_math.a")
            .unwrap();
        let runtime_index = args
            .iter()
            .position(|arg| arg == "/tmp/libmesh_rt.a")
            .unwrap();
        assert!(
            native_index < runtime_index,
            "unexpected linker args: {args:?}"
        );
        assert!(!args.iter().any(|arg| arg.contains("whole-archive")));
    }

    #[test]
    fn windows_clang_path_from_prefix_should_append_bin_clang_exe() {
        let actual = windows_clang_path_from_prefix(Path::new("C:/llvm"));
        assert_eq!(
            actual,
            PathBuf::from("C:/llvm").join("bin").join("clang.exe")
        );
    }

    fn find_mesh_rt_in(
        target_dirs: &[PathBuf],
        target: &LinkTarget,
        profiles: &[&str],
        runtime_flavor: RuntimeFlavor,
    ) -> Result<PathBuf, String> {
        let mut searched_paths = Vec::new();

        for target_dir in target_dirs {
            for candidate in mesh_rt_candidates(target_dir, target, profiles, runtime_flavor) {
                if candidate.exists() {
                    return Ok(candidate);
                }
                searched_paths.push(candidate);
            }
        }

        let mut message = format!(
            "Could not locate {} static library for target '{}'. Expected {}. Run `cargo build -p {}{}` first.",
            runtime_flavor.display_name(),
            target.display_triple(),
            target.runtime_filename(runtime_flavor),
            runtime_flavor.package_name(),
            target.cargo_build_hint(),
        );
        if !searched_paths.is_empty() {
            message.push_str("\nSearched:\n");
            for path in searched_paths {
                message.push_str("  - ");
                message.push_str(&path.display().to_string());
                message.push('\n');
            }
            message.pop();
        }

        Err(message)
    }

    fn unique_temp_target_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "mesh-codegen-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
