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
    // The build trace records the runtime this got to before `error`.
    let fail = |error: String, runtime: Option<&Path>| {
        let exists = runtime.is_some_and(Path::exists);
        build_trace::set_link_context(&target.display_triple(), runtime, Some(exists), None);
        build_trace::record_error(&error);
        error
    };

    let rt_path = match rt_lib_path {
        Some(path) => validate_runtime_override(path, &target, runtime_flavor)
            .map(|()| path.to_path_buf())
            .map_err(|error| fail(error, Some(path)))?,
        None => find_mesh_rt(&target, runtime_flavor).map_err(|error| fail(error, None))?,
    };
    let linker_program = target
        .linker_program()
        .map_err(|error| fail(error, Some(&rt_path)))?;
    let runtime_exists = rt_path.exists();
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

/// The `libtool` command that archives the program's object, the native
/// archives and the runtime into one static library for an Apple target.
fn apple_archive_command(object_path: &Path, output_path: &Path, plan: &LinkPlan) -> Command {
    let mut command = Command::new("xcrun");
    command.args(["libtool", "-static", "-o"]);
    command
        .arg(output_path)
        .arg(object_path)
        .args(&plan.native_archives)
        .arg(&plan.rt_path);
    command
}

pub(crate) fn archive_with_plan(
    object_path: &Path,
    output_path: &Path,
    plan: &LinkPlan,
) -> Result<(), String> {
    let output = if plan.target.is_apple() {
        apple_archive_command(object_path, output_path, plan).output()
    } else if plan.target.kind == LinkTargetKind::Unix {
        let script = archiver_script(object_path, output_path, plan);
        run_archiver(&plan.target.archiver_program()?, &script)
    } else {
        return Err("static library artifacts are not yet supported for Windows MSVC".to_string());
    }
    .map_err(|error| format!("Failed to create static library: {error}"))?;

    finish_library_link(output, object_path, output_path, "Static library creation")
}

/// Run `archiver -M` on the MRI `script`.
fn run_archiver(archiver: &Path, script: &str) -> std::io::Result<std::process::Output> {
    let mut child = Command::new(archiver)
        .arg("-M")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("the archiver's stdin is piped");
    stdin.write_all(script.as_bytes())?;
    drop(stdin);
    child.wait_with_output()
}

/// The MRI script `ar -M` builds a static library from: the program's
/// object, then every member of the native archives and of the runtime.
fn archiver_script(object_path: &Path, output_path: &Path, plan: &LinkPlan) -> String {
    let mut script = format!(
        "CREATE {}\nADDMOD {}\n",
        output_path.display(),
        object_path.display()
    );
    for archive in plan.native_archives.iter().chain([&plan.rt_path]) {
        script.push_str(&format!("ADDLIB {}\n", archive.display()));
    }
    script.push_str("SAVE\nEND\n");
    script
}

/// The name a host that links the library records for it: the file name
/// alone, found through its rpath or library path, rather than the path the
/// library was built at (`greeter/libgreeter.dylib` failed to load from any
/// other directory).
fn library_name_args(target: &LinkTarget, output_path: &Path) -> Vec<String> {
    let file_name = output_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if target.is_apple() {
        vec![format!("-Wl,-install_name,@rpath/{file_name}")]
    } else {
        vec![format!("-Wl,-soname,{file_name}")]
    }
}

pub(crate) fn link_dynamic_with_plan(
    object_path: &Path,
    output_path: &Path,
    plan: &LinkPlan,
) -> Result<(), String> {
    let output = dynamic_link_command(object_path, output_path, plan)?
        .output()
        .map_err(|error| format!("Failed to invoke dynamic linker: {error}"))?;
    finish_library_link(output, object_path, output_path, "Dynamic library linking")
}

/// The command that links the program's object into a dynamic library.
fn dynamic_link_command(
    object_path: &Path,
    output_path: &Path,
    plan: &LinkPlan,
) -> Result<Command, String> {
    if plan.target.kind == LinkTargetKind::WindowsMsvc {
        let mut command = build_link_command(object_path, output_path, plan);
        command.arg("-shared");
        // Export the host ABI as well as the generated dllexport wrappers.
        for symbol in [
            "mesh_library_init",
            "mesh_library_shutdown",
            "mesh_library_register_host_callbacks",
            "mesh_library_free_returned_bytes",
        ] {
            command.arg(format!("-Wl,/EXPORT:{symbol}"));
        }
        return Ok(command);
    }
    let mut command = plan.target.dynamic_linker_command()?;
    command.arg(object_path).args(&plan.native_archives);
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
    command.args(library_name_args(&plan.target, output_path));
    if plan.target.needs_security_framework() {
        for framework in ["Security", "CoreFoundation"] {
            command.arg("-framework").arg(framework);
        }
    }
    Ok(command)
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
            if !plan.target.is_apple() {
                // Drop the runtime sections a program never reaches, as rustc
                // does for Rust executables: a third off a debug link's time
                // and size on Linux.
                cmd.arg("-Wl,--gc-sections");
            }
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
    const PROFILES: [&str; 2] = if cfg!(debug_assertions) {
        ["debug", "release"]
    } else {
        ["release", "debug"]
    };

    let workspace_candidates = workspace_target_dirs()
        .into_iter()
        .flat_map(|target_dir| mesh_rt_candidates(&target_dir, target, &PROFILES, runtime_flavor));
    first_existing_runtime(
        installed_mesh_rt_candidates(target, runtime_flavor)
            .into_iter()
            .chain(workspace_candidates),
        target,
        runtime_flavor,
    )
}

/// The first of `candidates` that exists, or an error naming each.
fn first_existing_runtime(
    candidates: impl IntoIterator<Item = PathBuf>,
    target: &LinkTarget,
    runtime_flavor: RuntimeFlavor,
) -> Result<PathBuf, String> {
    let mut searched = String::new();
    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
        searched.push_str(&format!("\n  - {}", candidate.display()));
    }

    Err(format!(
        "Could not locate {} static library for target '{}'. Expected {} in the `lib` directory beside the installed meshc; reinstall Mesh, or in a source checkout run `cargo build -p {}{}` first.\nSearched:{searched}",
        runtime_flavor.display_name(),
        target.display_triple(),
        target.runtime_filename(runtime_flavor),
        runtime_flavor.package_name(),
        target.cargo_build_hint(),
    ))
}

/// Where an installed toolchain keeps the runtime: `<prefix>/lib/` beside
/// `<prefix>/bin/meshc`, as the installers lay it out (`~/.mesh/lib`). A
/// runtime for another target goes in `<prefix>/lib/<triple>/`.
fn installed_mesh_rt_candidates(
    target: &LinkTarget,
    runtime_flavor: RuntimeFlavor,
) -> Vec<PathBuf> {
    std::env::current_exe()
        .ok()
        .map(|exe| std::fs::canonicalize(&exe).unwrap_or(exe))
        .and_then(|exe| Some(exe.parent()?.parent()?.to_path_buf()))
        .map(|prefix| installed_runtime_candidates_under(&prefix, target, runtime_flavor))
        .unwrap_or_default()
}

fn installed_runtime_candidates_under(
    prefix: &Path,
    target: &LinkTarget,
    runtime_flavor: RuntimeFlavor,
) -> Vec<PathBuf> {
    let lib = prefix.join("lib");
    let file = target.runtime_filename(runtime_flavor);
    match target.requested_triple.as_deref() {
        Some(triple) if triple != host_target_triple() => vec![lib.join(triple).join(file)],
        _ => vec![lib.join(file)],
    }
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
            LinkTargetKind::WindowsMsvc => {
                windows_clang_path(std::env::var("LLVM_SYS_211_PREFIX").ok())
            }
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
        match self.requested_triple.as_deref() {
            Some(triple) if triple.contains("linux-android") => android_tool("llvm-ar"),
            _ => Ok(PathBuf::from("ar")),
        }
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
            let clang_name = format!("{}26-clang", ndk_clang_triple(triple));
            return android_tool(&clang_name).map(Command::new);
        }
        let mut command = Command::new("cc");
        command.args(["-target", triple]);
        Ok(command)
    }
}

/// The NDK names its 32-bit ARM compiler `armv7a-...`, where Rust's triple
/// says `armv7-` (or `thumbv7neon-`).
fn ndk_clang_triple(triple: &str) -> String {
    for rust_arch in ["armv7-", "thumbv7neon-"] {
        if let Some(rest) = triple.strip_prefix(rust_arch) {
            return format!("armv7a-{rest}");
        }
    }
    triple.to_string()
}

fn android_tool(name: &str) -> Result<PathBuf, String> {
    let ndk = std::env::var_os("ANDROID_NDK_HOME").or_else(|| std::env::var_os("ANDROID_NDK_ROOT"));
    android_tool_in(ndk.map(PathBuf::from), name)
}

/// The NDK tool `name` in the NDK at `ndk`.
fn android_tool_in(ndk: Option<PathBuf>, name: &str) -> Result<PathBuf, String> {
    let ndk = ndk.ok_or("Android target requires ANDROID_NDK_HOME or ANDROID_NDK_ROOT")?;
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

/// The linker family of the host meshc was built for, if it has one.
const HOST_LINK_TARGET: Option<LinkTargetKind> =
    if cfg!(all(target_os = "windows", target_env = "msvc")) {
        Some(LinkTargetKind::WindowsMsvc)
    } else if cfg!(target_family = "unix") {
        Some(LinkTargetKind::Unix)
    } else {
        None
    };

fn classify_host_target() -> Result<LinkTargetKind, String> {
    HOST_LINK_TARGET.ok_or_else(|| UNSUPPORTED_HOST.to_string())
}

const UNSUPPORTED_HOST: &str = "Unsupported host linker target: meshc links for Unix-like and Windows MSVC hosts; pass `--target` with a supported triple.";

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

/// The vendor and system parts of the host's target triple.
const HOST_VENDOR_SYSTEM: (&str, &str) = if cfg!(all(target_os = "windows", target_env = "msvc")) {
    ("pc", "windows-msvc")
} else if cfg!(target_os = "macos") {
    ("apple", "darwin")
} else if cfg!(target_os = "linux") {
    ("unknown", "linux-gnu")
} else {
    ("unknown", std::env::consts::OS)
};

fn host_target_triple() -> String {
    let (vendor, system) = HOST_VENDOR_SYSTEM;
    format!("{}-{vendor}-{system}", std::env::consts::ARCH)
}

/// The clang that links for Windows: the one in the LLVM at `llvm_prefix`
/// (`LLVM_SYS_211_PREFIX`), or else `clang` on the `PATH`.
fn windows_clang_path(llvm_prefix: Option<String>) -> Result<PathBuf, String> {
    let Some(prefix) = llvm_prefix else {
        return Ok(PathBuf::from("clang"));
    };
    let candidate = Path::new(&prefix).join("bin").join("clang.exe");
    if candidate.exists() {
        return Ok(candidate);
    }
    Err(format!(
        "LLVM_SYS_211_PREFIX='{}' does not contain bin/clang.exe at '{}'. Install LLVM 21 or set LLVM_SYS_211_PREFIX correctly.",
        prefix,
        candidate.display(),
    ))
}

/// The cargo target directories to look for the runtime in. An absolute
/// `CARGO_TARGET_DIR` names the one outright. Otherwise (a relative one is
/// relative to where cargo ran, not to meshc) they are the directories above
/// meshc, nearest first, that cargo tagged (whatever their name: a meshc
/// built with `CARGO_TARGET_DIR=target/other` links the runtime built beside
/// it), that are named `target`, or that hold one; the enclosing ones count
/// too, as cargo-llvm-cov builds meshc in `target/llvm-cov-target` and the
/// runtime in `target`.
fn workspace_target_dirs() -> Vec<PathBuf> {
    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
    {
        return vec![dir];
    }
    std::env::current_exe()
        .map(|exe| target_dirs_above(&exe))
        .unwrap_or_default()
}

fn target_dirs_above(path: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in path.ancestors().skip(1) {
        let candidate = if dir.join("CACHEDIR.TAG").is_file()
            || dir.file_name().is_some_and(|name| name == "target")
        {
            dir.to_path_buf()
        } else {
            dir.join("target")
        };
        if candidate.exists() && !dirs.contains(&candidate) {
            dirs.push(candidate);
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn workspace_target_dirs_are_found_during_cargo_test() {
        assert!(
            !workspace_target_dirs().is_empty(),
            "Should find workspace target dir during cargo test"
        );
    }

    #[test]
    fn target_dirs_start_with_the_tagged_one_meshc_was_built_in() {
        let root = std::env::temp_dir().join(format!("mesh-target-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let outer = root.join("target");
        let inner = outer.join("alt");
        std::fs::create_dir_all(inner.join("debug")).unwrap();
        std::fs::write(outer.join("CACHEDIR.TAG"), "").unwrap();
        std::fs::write(inner.join("CACHEDIR.TAG"), "").unwrap();
        // The temporary directory may itself be under a `target` directory.
        let within = |meshc: PathBuf| {
            target_dirs_above(&meshc)
                .into_iter()
                .filter(|dir| dir.starts_with(&root))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            within(inner.join("debug/meshc")),
            [inner.clone(), outer.clone()],
            "its own first, then the `target` around it"
        );
        std::fs::create_dir_all(root.join("custom/release")).unwrap();
        std::fs::write(root.join("custom/CACHEDIR.TAG"), "").unwrap();
        assert_eq!(
            within(root.join("custom/release/meshc")),
            [root.join("custom"), outer.clone()]
        );
        assert_eq!(
            within(root.join("bin/meshc")),
            [outer],
            "untagged: a `target` beside an ancestor"
        );
        std::fs::remove_dir_all(&root).unwrap();
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
    fn android_triples_name_the_ndk_compiler() {
        assert_eq!(
            ndk_clang_triple("aarch64-linux-android"),
            "aarch64-linux-android"
        );
        assert_eq!(
            ndk_clang_triple("x86_64-linux-android"),
            "x86_64-linux-android"
        );
        assert_eq!(
            ndk_clang_triple("armv7-linux-androideabi"),
            "armv7a-linux-androideabi"
        );
        assert_eq!(
            ndk_clang_triple("thumbv7neon-linux-androideabi"),
            "armv7a-linux-androideabi"
        );
    }

    #[test]
    fn dynamic_libraries_are_named_by_file_not_build_path() {
        let output = Path::new("greeter/libgreeter.dylib");
        let apple = LinkTarget::detect(Some("aarch64-apple-darwin")).unwrap();
        assert_eq!(
            library_name_args(&apple, output),
            ["-Wl,-install_name,@rpath/libgreeter.dylib"]
        );
        let linux = LinkTarget::detect(Some("x86_64-unknown-linux-gnu")).unwrap();
        assert_eq!(
            library_name_args(&linux, Path::new("out/libgreeter.so")),
            ["-Wl,-soname,libgreeter.so"]
        );
    }

    #[test]
    fn installed_runtime_is_found_in_lib_beside_bin() {
        let prefix = Path::new("/home/user/.mesh");
        let host = LinkTarget::detect(None).unwrap();
        assert_eq!(
            installed_runtime_candidates_under(prefix, &host, RuntimeFlavor::Test),
            vec![prefix
                .join("lib")
                .join(host.runtime_filename(RuntimeFlavor::Test))]
        );

        let named_host = LinkTarget::detect(Some(&host_target_triple())).unwrap();
        assert_eq!(
            installed_runtime_candidates_under(prefix, &named_host, RuntimeFlavor::Standard),
            vec![prefix
                .join("lib")
                .join(host.runtime_filename(RuntimeFlavor::Standard))]
        );

        let cross = LinkTarget::detect(Some("aarch64-linux-android")).unwrap();
        assert_eq!(
            installed_runtime_candidates_under(prefix, &cross, RuntimeFlavor::Standard),
            vec![prefix.join("lib/aarch64-linux-android/libmesh_rt.a")]
        );
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
            std::slice::from_ref(&temp_target),
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
            std::slice::from_ref(&temp_target),
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
            std::slice::from_ref(&temp_target),
            &target,
            &["debug", "release"],
            RuntimeFlavor::Test,
        )
        .unwrap();
        assert_eq!(found, runtime);

        fs::remove_file(&runtime).unwrap();
        let error = find_mesh_rt_in(
            std::slice::from_ref(&temp_target),
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
            std::slice::from_ref(&temp_target),
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
    fn elf_executables_drop_unreachable_sections() {
        let args = |triple: &str| {
            let plan = LinkPlan {
                target: LinkTarget::detect(Some(triple)).unwrap(),
                rt_path: PathBuf::from("/tmp/libmesh_rt.a"),
                linker_program: PathBuf::from("cc"),
                native_archives: Vec::new(),
            };
            build_link_command(Path::new("/tmp/main.o"), Path::new("/tmp/app"), &plan)
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert!(args("x86_64-unknown-linux-gnu").contains(&"-Wl,--gc-sections".to_string()));
        assert!(!args("aarch64-apple-darwin").contains(&"-Wl,--gc-sections".to_string()));
    }

    #[test]
    fn windows_links_with_the_clang_of_the_named_llvm_or_the_path() {
        assert_eq!(windows_clang_path(None), Ok(PathBuf::from("clang")));
        let llvm = unique_temp_target_dir("llvm-prefix");
        let missing = windows_clang_path(Some(llvm.display().to_string())).unwrap_err();
        assert!(
            missing.contains("does not contain bin/clang.exe"),
            "{missing}"
        );
        let clang = llvm.join("bin").join("clang.exe");
        fs::create_dir_all(clang.parent().unwrap()).unwrap();
        fs::write(&clang, b"").unwrap();
        assert_eq!(
            windows_clang_path(Some(llvm.display().to_string())),
            Ok(clang)
        );
        fs::remove_dir_all(llvm).unwrap();
    }

    #[test]
    fn android_tools_come_from_the_ndk_prebuilt_toolchain() {
        let error = android_tool_in(None, "llvm-ar").unwrap_err();
        assert!(error.contains("ANDROID_NDK_HOME"), "{error}");
        let ndk = unique_temp_target_dir("ndk");
        let error = android_tool_in(Some(ndk.clone()), "llvm-ar").unwrap_err();
        assert!(error.contains("is unavailable"), "{error}");
        let bin = ndk.join("toolchains/llvm/prebuilt/host/bin");
        fs::create_dir_all(&bin).unwrap();
        let error = android_tool_in(Some(ndk.clone()), "llvm-ar").unwrap_err();
        assert!(error.contains("'llvm-ar' was not found"), "{error}");
        fs::write(bin.join("llvm-ar"), b"").unwrap();
        assert_eq!(
            android_tool_in(Some(ndk.clone()), "llvm-ar"),
            Ok(bin.join("llvm-ar"))
        );
        fs::remove_dir_all(ndk).unwrap();

        // Android targets archive and link with the NDK's tools.
        let android = LinkTarget::detect(Some("aarch64-linux-android")).unwrap();
        assert_eq!(android.archiver_program(), android_tool("llvm-ar"));
        assert_eq!(
            android
                .dynamic_linker_command()
                .map(|c| c.get_program().to_owned()),
            android_tool("aarch64-linux-android26-clang").map(Into::into)
        );
    }

    #[test]
    fn each_target_has_its_archiver_and_dynamic_linker() {
        assert_eq!(
            LinkTarget::detect(None).unwrap().archiver_program(),
            Ok(PathBuf::from("ar"))
        );
        let linux = LinkTarget::detect(Some("x86_64-unknown-linux-gnu")).unwrap();
        assert_eq!(linux.archiver_program(), Ok(PathBuf::from("ar")));
        let command = |triple: &str| {
            let command = LinkTarget::detect(Some(triple))
                .unwrap()
                .dynamic_linker_command()
                .unwrap();
            let args = command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned());
            std::iter::once(command.get_program().to_string_lossy().into_owned())
                .chain(args)
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            command("aarch64-apple-ios"),
            "xcrun --sdk iphoneos clang -target aarch64-apple-ios"
        );
        assert_eq!(
            command("aarch64-apple-ios-sim"),
            "xcrun --sdk iphonesimulator clang -target aarch64-apple-ios-sim"
        );
        assert_eq!(
            command("x86_64-apple-darwin"),
            "xcrun --sdk macosx clang -target x86_64-apple-darwin"
        );
        assert_eq!(
            command("x86_64-unknown-linux-musl"),
            "cc -target x86_64-unknown-linux-musl"
        );
    }

    #[test]
    fn dynamic_libraries_link_the_whole_runtime_and_export_the_host_abi() {
        let args = |triple: &str| {
            dynamic_link_command(
                Path::new("/tmp/lib.o"),
                Path::new("/tmp/out/libapp.so"),
                &plan_for(triple),
            )
            .unwrap()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
        };
        let windows = args("x86_64-pc-windows-msvc");
        assert!(windows.contains(&"-shared".to_string()), "{windows:?}");
        assert!(
            windows.contains(&"-Wl,/EXPORT:mesh_library_init".to_string()),
            "{windows:?}"
        );
        let linux = args("x86_64-unknown-linux-gnu");
        let whole = linux
            .iter()
            .position(|arg| arg == "-Wl,--whole-archive")
            .unwrap();
        assert_eq!(linux[whole + 1], "/tmp/libmesh_rt.a");
        assert!(
            linux.contains(&"-Wl,-soname,libapp.so".to_string()),
            "{linux:?}"
        );
        let apple = args("aarch64-apple-darwin");
        assert!(
            apple.contains(&"-Wl,-force_load,/tmp/libmesh_rt.a".to_string()),
            "{apple:?}"
        );
        assert!(apple.contains(&"Security".to_string()), "{apple:?}");
    }

    /// A package's native archives go into a library beside the program's
    /// object, ahead of the runtime: archived with it for an Apple static
    /// library, linked with it into a dynamic one.
    #[test]
    fn libraries_take_the_native_archives_before_the_runtime() {
        let plan = LinkPlan {
            native_archives: vec![PathBuf::from("/tmp/libnative.a")],
            ..plan_for("aarch64-apple-darwin")
        };
        let args = |command: Command| {
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        let archived = args(apple_archive_command(
            Path::new("/tmp/lib.o"),
            Path::new("/tmp/libapp.a"),
            &plan,
        ));
        assert_eq!(
            archived,
            [
                "libtool",
                "-static",
                "-o",
                "/tmp/libapp.a",
                "/tmp/lib.o",
                "/tmp/libnative.a",
                "/tmp/libmesh_rt.a"
            ]
        );
        let linked = args(
            dynamic_link_command(
                Path::new("/tmp/lib.o"),
                Path::new("/tmp/libapp.dylib"),
                &plan,
            )
            .unwrap(),
        );
        let native = linked.iter().position(|arg| arg == "/tmp/libnative.a");
        let runtime = linked
            .iter()
            .position(|arg| arg == "-Wl,-force_load,/tmp/libmesh_rt.a");
        assert!(native.unwrap() < runtime.unwrap(), "{linked:?}");
    }

    #[cfg(unix)]
    #[test]
    fn static_libraries_are_archived_from_an_mri_script() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_temp_target_dir("archiver");
        let plan = LinkPlan {
            native_archives: vec![dir.join("libnative.a")],
            ..plan_for("x86_64-unknown-linux-gnu")
        };
        let script = archiver_script(&dir.join("lib.o"), &dir.join("libapp.a"), &plan);
        assert_eq!(
            script,
            format!(
                "CREATE {0}/libapp.a\nADDMOD {0}/lib.o\nADDLIB {0}/libnative.a\nADDLIB /tmp/libmesh_rt.a\nSAVE\nEND\n",
                dir.display()
            )
        );

        // The archiver reads the script from its input.
        let archiver = dir.join("fake-ar");
        let received = dir.join("received");
        fs::write(
            &archiver,
            format!("#!/bin/sh\ncat > '{}'\n", received.display()),
        )
        .unwrap();
        fs::set_permissions(&archiver, fs::Permissions::from_mode(0o755)).unwrap();
        let output = run_archiver(&archiver, &script).unwrap();
        assert!(output.status.success());
        assert_eq!(fs::read_to_string(&received).unwrap(), script);

        // A failed archive says why; Windows has none yet.
        let error =
            archive_with_plan(&dir.join("missing.o"), &dir.join("libapp.a"), &plan).unwrap_err();
        assert!(error.to_lowercase().contains("static library"), "{error}");
        let windows = plan_for("x86_64-pc-windows-msvc");
        let error =
            archive_with_plan(&dir.join("lib.o"), &dir.join("app.lib"), &windows).unwrap_err();
        assert!(
            error.contains("not yet supported for Windows MSVC"),
            "{error}"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_finished_library_link_removes_its_object_or_says_what_failed() {
        use std::process::Output;

        #[cfg(unix)]
        let status = |code: i32| std::os::unix::process::ExitStatusExt::from_raw(code << 8);
        #[cfg(windows)]
        let status = |code: u32| std::os::windows::process::ExitStatusExt::from_raw(code);
        let dir = unique_temp_target_dir("finish-library");
        let object = dir.join("lib.o");
        fs::write(&object, b"").unwrap();
        let output = |code, stdout: &str, stderr: &str| Output {
            status: status(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let lib = dir.join("libapp.a");
        let error =
            finish_library_link(output(1, "out", " err "), &object, &lib, "Linking").unwrap_err();
        assert!(error.ends_with("libapp.a': err"), "{error}");
        let error =
            finish_library_link(output(1, " out ", ""), &object, &lib, "Linking").unwrap_err();
        assert!(error.ends_with("libapp.a': out"), "{error}");
        assert!(object.exists());
        finish_library_link(output(0, "", ""), &object, &lib, "Linking").unwrap();
        assert!(!object.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn runtime_and_native_archive_paths_are_checked_before_linking() {
        let unix = LinkTarget::detect(Some("x86_64-unknown-linux-gnu")).unwrap();
        let windows = LinkTarget::detect(Some("x86_64-pc-windows-msvc")).unwrap();
        assert_eq!(
            windows.runtime_filename(RuntimeFlavor::Test),
            "mesh_test_rt.lib"
        );
        let error =
            validate_runtime_override(Path::new("/"), &unix, RuntimeFlavor::Standard).unwrap_err();
        assert!(error.contains("does not name a file"), "{error}");

        let dir = unique_temp_target_dir("native-archives");
        let relative = validate_native_archive(Path::new("libnative.a"), &unix).unwrap_err();
        assert!(
            relative.contains("existing absolute file path"),
            "{relative}"
        );
        let missing = validate_native_archive(&dir.join("libnative.a"), &unix).unwrap_err();
        assert!(missing.contains("existing absolute file path"), "{missing}");
        let archive = dir.join("libnative.a");
        fs::write(&archive, b"").unwrap();
        assert_eq!(validate_native_archive(&archive, &unix), Ok(()));
        let error = validate_native_archive(&archive, &windows).unwrap_err();
        assert!(error.contains("`.lib` static-library extension"), "{error}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unsupported_targets_are_refused() {
        let error = classify_requested_target("wasm32-unknown-unknown").unwrap_err();
        assert!(error.contains("Supported linker families"), "{error}");
    }

    #[test]
    fn a_missing_runtime_lists_where_it_was_looked_for() {
        let target = LinkTarget::detect(Some("x86_64-unknown-linux-gnu")).unwrap();
        let error = first_existing_runtime(
            [PathBuf::from("/nowhere/a"), PathBuf::from("/nowhere/b")],
            &target,
            RuntimeFlavor::Standard,
        )
        .unwrap_err();
        assert!(
            error.ends_with("first.\nSearched:\n  - /nowhere/a\n  - /nowhere/b"),
            "{error}"
        );
    }

    #[test]
    fn a_compiler_outside_any_target_directory_has_none() {
        assert!(target_dirs_above(Path::new("/meshc")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_linker_that_fails_quietly_or_on_stdout_is_reported() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_temp_target_dir("quiet-linker");
        let link_error = |script: &str| {
            let linker = dir.join("linker");
            fs::write(&linker, script).unwrap();
            fs::set_permissions(&linker, fs::Permissions::from_mode(0o755)).unwrap();
            let plan = LinkPlan {
                linker_program: linker,
                ..plan_for("x86_64-unknown-linux-gnu")
            };
            link_with_plan(&dir.join("main.o"), &dir.join("app"), &plan).unwrap_err()
        };
        let error = link_error("#!/bin/sh\necho undefined symbol\nexit 1\n");
        assert!(error.ends_with("stdout:\nundefined symbol"), "{error}");
        let error = link_error("#!/bin/sh\nexit 1\n");
        assert!(error.contains("without emitting output"), "{error}");
        fs::remove_dir_all(dir).unwrap();
    }

    fn find_mesh_rt_in(
        target_dirs: &[PathBuf],
        target: &LinkTarget,
        profiles: &[&str],
        runtime_flavor: RuntimeFlavor,
    ) -> Result<PathBuf, String> {
        first_existing_runtime(
            target_dirs
                .iter()
                .flat_map(|dir| mesh_rt_candidates(dir, target, profiles, runtime_flavor)),
            target,
            runtime_flavor,
        )
    }

    fn plan_for(triple: &str) -> LinkPlan {
        LinkPlan {
            target: LinkTarget::detect(Some(triple)).unwrap(),
            rt_path: PathBuf::from("/tmp/libmesh_rt.a"),
            linker_program: PathBuf::from("cc"),
            native_archives: Vec::new(),
        }
    }

    fn link_args(plan: &LinkPlan) -> Vec<String> {
        build_link_command(Path::new("/tmp/main.o"), Path::new("/tmp/app"), plan)
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn each_target_links_its_own_system_libraries() {
        let windows = link_args(&plan_for("x86_64-pc-windows-msvc"));
        assert!(
            windows.contains(&"-Wl,ws2_32.lib".to_string()),
            "{windows:?}"
        );
        assert!(windows.contains(&"-v".to_string()), "{windows:?}");
        assert!(!windows.contains(&"-lm".to_string()), "{windows:?}");
        let apple = link_args(&plan_for("aarch64-apple-darwin"));
        let framework = apple.iter().position(|arg| arg == "-framework").unwrap();
        assert_eq!(apple[framework + 1], "Security");
        let linux = link_args(&plan_for("x86_64-unknown-linux-gnu"));
        assert!(!linux.contains(&"-framework".to_string()), "{linux:?}");
        assert!(!LinkTarget::detect(Some("x86_64-pc-windows-msvc"))
            .unwrap()
            .linker_help_suffix()
            .is_empty());
    }

    /// Why a link could not start or failed, named for what went wrong.
    #[test]
    fn a_link_that_cannot_run_or_fails_says_why() {
        let override_error = prepare_link_for_runtime(
            None,
            Some(Path::new("/nowhere/libwrong.a")),
            &[],
            RuntimeFlavor::Standard,
        )
        .unwrap_err();
        assert!(
            override_error.contains("does not match expected filename"),
            "{override_error}"
        );
        let missing = prepare_link_for_runtime(
            None,
            Some(Path::new("/nowhere/libmesh_rt.a")),
            &[],
            RuntimeFlavor::Standard,
        )
        .unwrap_err();
        assert!(
            missing.contains("static library not found at '/nowhere/libmesh_rt.a'"),
            "{missing}"
        );
        assert!(missing.contains("cargo build -p mesh-rt"), "{missing}");

        let dir = unique_temp_target_dir("failing-link");
        let host = LinkTarget::detect(None).unwrap();
        let plan = LinkPlan {
            linker_program: dir.join("no-such-linker"),
            ..plan_for(&host.display_triple())
        };
        let error = link_with_plan(&dir.join("main.o"), &dir.join("app"), &plan).unwrap_err();
        assert!(error.contains("Failed to invoke linker"), "{error}");
        let plan = LinkPlan {
            linker_program: PathBuf::from("cc"),
            ..plan
        };
        let error = link_with_plan(&dir.join("missing.o"), &dir.join("app"), &plan).unwrap_err();
        assert!(error.contains("Linking failed for target"), "{error}");
        assert!(error.contains("stderr:"), "{error}");
        fs::remove_dir_all(dir).unwrap();
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
