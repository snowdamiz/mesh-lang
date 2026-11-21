use mesh_diagnostics::{Code, Diagnostic, Failed, Reporter};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::{builder::PossibleValuesParser, Subcommand};
use serde::{Deserialize, Serialize};

use crate::guide;

const RECEIPT: &str = ".mesh-install.json";
const BACKUP: &str = ".mesh-skill-previous";

#[derive(Subcommand)]
pub(crate) enum SkillsCommand {
    /// Choose coding tools and install their Mesh skill
    Setup {
        /// Approve installation in the current project without prompting
        #[arg(long, short = 'y')]
        yes: bool,

        /// Coding tool to install for (repeat to select multiple tools)
        #[arg(long, value_name = "AGENT", value_parser = PossibleValuesParser::new(HOSTS.iter().map(|host| host.id)))]
        target: Vec<String>,
    },
    /// Refresh account skills previously installed by Mesh
    Refresh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Scope {
    Account,
    Project,
}

struct Host {
    id: &'static str,
    name: &'static str,
    account: &'static str,
    project: &'static str,
}

const HOSTS: &[Host] = &[
    Host {
        id: "codex",
        name: "Codex",
        account: ".agents",
        project: ".agents",
    },
    Host {
        id: "claude-code",
        name: "Claude Code",
        account: ".claude",
        project: ".claude",
    },
    Host {
        id: "cursor",
        name: "Cursor",
        account: ".cursor",
        project: ".agents",
    },
    Host {
        id: "gemini-cli",
        name: "Gemini CLI",
        account: ".gemini",
        project: ".gemini",
    },
    Host {
        id: "github-copilot-cli",
        name: "GitHub Copilot CLI",
        account: ".copilot",
        project: ".github",
    },
];

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u8,
    scope: Scope,
    digest: String,
}

pub(crate) fn run(command: SkillsCommand, reporter: &mut Reporter) -> Result<(), Failed> {
    let report_io = |error: io::Error| Diagnostic::new(Code::PathNotFound, error.to_string());
    let (selection, setup) = match command {
        SkillsCommand::Setup { yes: true, target } if !target.is_empty() => {
            let hosts = HOSTS
                .iter()
                .filter(|host| target.iter().any(|id| id == host.id));
            let root = project_root().map_err(|error| reporter.fail(report_io(error)))?;
            (Some(Selection::new(Scope::Project, root, hosts)), true)
        }
        SkillsCommand::Setup { yes: false, target }
            if target.is_empty() && io::stdin().is_terminal() =>
        {
            let selection = select(&mut io::stdin().lock(), &mut io::stdout().lock())
                .map_err(|error| reporter.fail(report_io(error)))?;
            (selection, true)
        }
        SkillsCommand::Setup { .. } => {
            return Err(reporter.fail(Diagnostic::new(
                Code::SkillsSetupNeedsTarget,
                "Skill setup needs both --yes and --target to run without prompts",
            )));
        }
        SkillsCommand::Refresh => {
            let root = account_home().map_err(|error| reporter.fail(report_io(error)))?;
            (
                Some(Selection::new(Scope::Account, root, HOSTS.iter())),
                false,
            )
        }
    };
    if let Some(selection) = selection {
        apply(&selection, setup, reporter)?;
    }
    Ok(())
}

struct Selection {
    scope: Scope,
    root: PathBuf,
    destinations: BTreeSet<&'static str>,
}

impl Selection {
    fn new(scope: Scope, root: PathBuf, hosts: impl Iterator<Item = &'static Host>) -> Self {
        let destinations = hosts
            .map(|host| match scope {
                Scope::Account => host.account,
                Scope::Project => host.project,
            })
            .collect();
        Self {
            scope,
            root,
            destinations,
        }
    }
}

fn project_root() -> io::Result<PathBuf> {
    let cwd = env::current_dir()?;
    Ok(cwd
        .ancestors()
        .find(|path| path.join("mesh.toml").is_file() || path.join(".git").exists())
        .unwrap_or(&cwd)
        .to_owned())
}

fn ask(
    input: &mut impl BufRead,
    output: &mut impl Write,
    question: &str,
) -> io::Result<Option<String>> {
    write!(output, "{question}")?;
    output.flush()?;
    let mut line = String::new();
    Ok((input.read_line(&mut line)? != 0).then(|| line.trim().to_owned()))
}

fn select(input: &mut impl BufRead, output: &mut impl Write) -> io::Result<Option<Selection>> {
    writeln!(
        output,
        "Choose your coding tools. Enter one or more numbers separated by commas."
    )?;
    for (index, host) in HOSTS.iter().enumerate() {
        writeln!(output, "  {}. {}", index + 1, host.name)?;
    }
    let hosts = loop {
        let Some(line) = ask(input, output, "Tools (Enter to cancel): ")? else {
            return Ok(None);
        };
        if line.is_empty() {
            return Ok(None);
        }
        let hosts = line
            .split(',')
            .map(|value| {
                value
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .and_then(|number| number.checked_sub(1))
                    .and_then(|index| HOSTS.get(index))
            })
            .collect::<Option<Vec<_>>>();
        if let Some(hosts) = hosts {
            break hosts;
        }
        writeln!(output, "Enter numbers from 1 through {}.", HOSTS.len())?;
    };

    let project = project_root()?;
    writeln!(output, "  1. Account (for all projects)")?;
    writeln!(output, "  2. Project ({})", project.display())?;
    let (scope, root) = loop {
        match ask(input, output, "Scope (Enter to cancel): ")?.as_deref() {
            Some("1") => break (Scope::Account, account_home()?),
            Some("2") => break (Scope::Project, project),
            Some("") | None => return Ok(None),
            _ => writeln!(output, "Enter 1 or 2.")?,
        }
    };
    let selection = Selection::new(scope, root, hosts.into_iter());
    writeln!(output, "Mesh will install its skill in these directories:")?;
    for host_dir in &selection.destinations {
        writeln!(
            output,
            "  {}",
            skill_path(&selection.root, host_dir).display()
        )?;
    }
    if selection.destinations.contains(".agents") {
        writeln!(
            output,
            "Other coding tools that read .agents/skills can also discover this copy."
        )?;
    }
    let answer = ask(input, output, "Install the Mesh skill? [y/N] ")?;
    if !answer.is_some_and(|answer| matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes")) {
        writeln!(output, "Setup skipped.")?;
        return Ok(None);
    }
    Ok(Some(selection))
}

fn account_home() -> io::Result<PathBuf> {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            invalid(format!(
                "Set {name} to the absolute path to your home directory."
            ))
        })
}

fn skill_path(root: &Path, host_dir: &str) -> PathBuf {
    root.join(host_dir).join("skills").join("mesh")
}

fn apply(
    selection: &Selection,
    setup: bool,
    reporter: &mut Reporter<impl Write>,
) -> Result<(), Failed> {
    let receipt = Receipt {
        schema: 1,
        scope: selection.scope,
        digest: guide::digest(guide::files()),
    };
    let mut failure = None;
    for host_dir in &selection.destinations {
        let path = skill_path(&selection.root, host_dir);
        match update(&selection.root, host_dir, &path, &receipt, setup) {
            Ok(Some(message)) => reporter.status(format_args!("{message} {}", path.display())),
            Ok(None) => {}
            Err(error) => {
                let diagnostic = match error.kind() {
                    io::ErrorKind::InvalidData | io::ErrorKind::AlreadyExists => Diagnostic::new(
                        Code::SkillsInstallConflict,
                        format!("Kept {} ({error}).", path.display()),
                    ),
                    _ => Diagnostic::new(
                        Code::PathNotFound,
                        format!("Could not update {} ({error}).", path.display()),
                    ),
                };
                failure = Some(reporter.fail(diagnostic));
            }
        }
    }
    failure.map_or(Ok(()), Err)
}

fn update(
    root: &Path,
    host_dir: &str,
    path: &Path,
    receipt: &Receipt,
    setup: bool,
) -> io::Result<Option<&'static str>> {
    let host = root.join(host_dir);
    let parent = path.parent().unwrap();
    for directory in [&*host, parent] {
        match metadata(directory)? {
            Some(meta) if ordinary(&meta) && meta.is_dir() => {}
            Some(_) => {
                return Err(invalid(format!(
                    "{} is not an ordinary directory",
                    directory.display()
                )))
            }
            None if !setup => return Ok(None),
            None => match fs::create_dir(directory) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => require_dir(directory)?,
                Err(err) => return Err(err),
            },
        }
    }

    let backup = parent.join(BACKUP);
    let pending = metadata(&backup)?.is_some();
    if !pending {
        match installed(path)? {
            None if !setup => return Ok(None),
            Some(old) if old == *receipt => return Ok(setup.then_some("Already installed")),
            _ => {}
        }
    }

    // Serialize Mesh writers. The OS releases the lock if an installer is interrupted.
    let lock_path = parent.join(".mesh-skill.lock");
    if metadata(&lock_path)?.is_some_and(|meta| !ordinary(&meta) || !meta.is_file()) {
        return Err(invalid(format!(
            "{} is not an ordinary file",
            lock_path.display()
        )));
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.lock()?;

    recover(path, &backup, receipt.scope)?;
    let previous = installed(path)?;
    if let Some(old) = &previous {
        if old.scope != receipt.scope {
            return Err(invalid("installed with a different scope"));
        }
        if old == receipt {
            return Ok(setup.then_some("Already installed"));
        }
    } else if !setup {
        return Ok(None);
    }

    let stage = tempfile::Builder::new()
        .prefix(".mesh-skill-stage-")
        .tempdir_in(parent)?;
    let staged = stage.path().join("mesh");
    fs::create_dir(&staged)?;
    fs::create_dir(staged.join("references"))?;
    for (name, bytes) in guide::files() {
        write_new(&staged.join(name), bytes)?;
    }
    write_new(&staged.join(RECEIPT), &serde_json::to_vec(receipt)?)?;

    if installed(path)? != previous {
        return Err(invalid("skill changed during setup"));
    }
    let Some(previous) = previous else {
        move_into_place(&staged, path, false)?;
        return Ok(Some("Installed"));
    };
    replace(&staged, path, &backup, receipt.scope)?;
    if staged.exists() && !matches!(installed(&staged), Ok(Some(old)) if old == previous) {
        let saved = stage.keep().join("mesh");
        return Err(io::Error::other(format!(
            "the previous copy changed during setup and is saved at {}",
            saved.display()
        )));
    }
    Ok(Some("Updated"))
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create_new(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn ordinary(meta: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & REPARSE_POINT != 0 {
            return false;
        }
    }
    !meta.is_symlink()
}

fn metadata(path: &Path) -> io::Result<Option<Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn require_dir(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if ordinary(&meta) && meta.is_dir() {
        Ok(())
    } else {
        Err(invalid(format!(
            "{} is not an ordinary directory",
            path.display()
        )))
    }
}

fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    if ordinary(&meta) && meta.is_file() {
        fs::read(path)
    } else {
        Err(invalid(format!(
            "{} is not an ordinary file",
            path.display()
        )))
    }
}

fn installed(path: &Path) -> io::Result<Option<Receipt>> {
    let Some(meta) = metadata(path)? else {
        return Ok(None);
    };
    if !ordinary(&meta) || !meta.is_dir() {
        return Err(invalid("skill is not an ordinary directory"));
    }
    let receipt_path = path.join(RECEIPT);
    if metadata(&receipt_path)?.is_none() {
        return Err(invalid("skill was not installed by Mesh"));
    }
    let receipt: Receipt = serde_json::from_slice(&read_file(&receipt_path)?)
        .map_err(|_| invalid("unrecognized Mesh skill receipt"))?;
    if receipt.schema != 1 {
        return Err(invalid("unrecognized Mesh skill receipt"));
    }

    let mut files = BTreeMap::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        match name.to_str() {
            Some("SKILL.md") => {
                files.insert("SKILL.md".to_owned(), read_file(&entry.path())?);
            }
            Some("references") => {
                require_dir(&entry.path())?;
                for reference in fs::read_dir(entry.path())? {
                    let reference = reference?;
                    let name = reference
                        .file_name()
                        .into_string()
                        .map_err(|_| invalid("unrecognized skill file"))?;
                    files.insert(format!("references/{name}"), read_file(&reference.path())?);
                }
            }
            Some(RECEIPT) => {}
            _ => return Err(invalid("skill contains extra files")),
        }
    }

    let digest = guide::digest(
        files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice())),
    );
    if digest != receipt.digest {
        return Err(invalid("skill files differ from the installed copy"));
    }
    Ok(Some(receipt))
}

fn recover(path: &Path, backup: &Path, scope: Scope) -> io::Result<()> {
    let Some(previous) = installed(backup)? else {
        return Ok(());
    };
    if previous.scope != scope {
        return Err(invalid(format!(
            "different scope in backup {}",
            backup.display()
        )));
    }
    if installed(path)?.is_some_and(|current| current.scope == scope) {
        fs::remove_dir_all(backup)
    } else {
        move_into_place(backup, path, false)
    }
}

fn replace(staged: &Path, live: &Path, backup: &Path, scope: Scope) -> io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    match move_into_place(staged, live, true) {
        Ok(()) => return Ok(()),
        Err(error)
            if !matches!(
                error.raw_os_error(),
                Some(libc::ENOTSUP | libc::EINVAL | libc::ENOSYS)
            ) =>
        {
            return Err(error)
        }
        Err(_) => {}
    }

    // Windows and filesystems without exchange keep the complete old tree
    // under a recovery name until the new tree is in place.
    move_into_place(live, backup, false)?;
    if let Err(error) = move_into_place(staged, live, false) {
        recover(live, backup, scope)?;
        return Err(error);
    }
    recover(live, backup, scope)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn move_into_place(from: &Path, to: &Path, exchange: bool) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    #[cfg(target_os = "macos")]
    let flags = if exchange {
        libc::RENAME_SWAP
    } else {
        libc::RENAME_EXCL
    };
    #[cfg(target_os = "linux")]
    let flags = if exchange {
        libc::RENAME_EXCHANGE
    } else {
        libc::RENAME_NOREPLACE
    };

    // Each pointer is a live, NUL-terminated path for the duration of the call.
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), flags) };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        // musl's libc bindings do not expose the renameat2 wrapper.
        libc::syscall(
            libc::SYS_renameat2,
            libc::c_long::from(libc::AT_FDCWD),
            from.as_ptr(),
            libc::c_long::from(libc::AT_FDCWD),
            to.as_ptr(),
            flags as libc::c_long,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn move_into_place(from: &Path, to: &Path, exchange: bool) -> io::Result<()> {
    debug_assert!(!exchange);
    if metadata(to)?.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "skill destination already exists",
        ));
    }
    fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(scope: Scope) -> Receipt {
        Receipt {
            schema: 1,
            scope,
            digest: guide::digest(guide::files()),
        }
    }

    fn old_copy(path: &Path, scope: Scope) -> Receipt {
        let content = b"# Older Mesh guide\n";
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("SKILL.md"), content).unwrap();
        let old = Receipt {
            schema: 1,
            scope,
            digest: guide::digest([("SKILL.md", content.as_slice())]),
        };
        fs::write(path.join(RECEIPT), serde_json::to_vec(&old).unwrap()).unwrap();
        old
    }

    #[test]
    fn refresh_updates_only_enrolled_unmodified_account_copies() {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join("project");
        let second_project = home.path().join("second-project");
        let clean = skill_path(home.path(), ".agents");
        let edited = skill_path(home.path(), ".claude");
        let extra = skill_path(home.path(), ".cursor");
        let pinned = skill_path(&project, ".agents");
        let pinned_current = skill_path(&second_project, ".agents");
        old_copy(&clean, Scope::Account);
        old_copy(&edited, Scope::Account);
        old_copy(&extra, Scope::Account);
        let old_project = old_copy(&pinned, Scope::Project);
        let current_project = receipt(Scope::Project);
        fs::create_dir(&second_project).unwrap();
        update(
            &second_project,
            ".agents",
            &pinned_current,
            &current_project,
            true,
        )
        .unwrap();
        fs::write(edited.join("SKILL.md"), "Local instructions").unwrap();
        fs::write(extra.join("notes.md"), "Local notes").unwrap();

        let selection = Selection::new(Scope::Account, home.path().to_owned(), HOSTS.iter());
        let mut output = Vec::new();
        let mut reporter = Reporter::new(mesh_diagnostics::Format::Json, &mut output);
        assert!(apply(&selection, false, &mut reporter).is_err());
        assert_eq!(reporter.finish().errors, 2);

        assert_eq!(installed(&clean).unwrap(), Some(receipt(Scope::Account)));
        assert_eq!(
            fs::read_to_string(edited.join("SKILL.md")).unwrap(),
            "Local instructions"
        );
        assert_eq!(
            fs::read_to_string(extra.join("notes.md")).unwrap(),
            "Local notes"
        );
        assert_eq!(installed(&pinned).unwrap(), Some(old_project));
        assert_eq!(installed(&pinned_current).unwrap(), Some(current_project));
        assert!(!home.path().join(".gemini").exists());
        let output = String::from_utf8(output).unwrap();
        let kept: Vec<_> = output
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect();
        assert!(kept.iter().all(|record| record["code"] == "X0103"));
        for path in [edited, extra] {
            assert!(kept.iter().any(|record| record["message"]
                .as_str()
                .unwrap()
                .contains(&format!("Kept {}", path.display()))));
        }

        let project_receipt = receipt(Scope::Project);
        assert_eq!(
            update(&project, ".agents", &pinned, &project_receipt, true).unwrap(),
            Some("Updated")
        );
        assert_eq!(installed(&pinned).unwrap(), Some(project_receipt));
        assert_eq!(
            update(&project, ".agents", &pinned, &receipt(Scope::Project), true).unwrap(),
            Some("Already installed")
        );
    }

    #[test]
    fn restores_the_previous_copy_if_replacement_was_interrupted() {
        let home = tempfile::tempdir().unwrap();
        let path = skill_path(home.path(), ".claude");
        let backup = path.with_file_name(BACKUP);
        let old = old_copy(&backup, Scope::Account);
        assert!(!path.exists());
        recover(&path, &backup, Scope::Account).unwrap();
        assert_eq!(installed(&path).unwrap(), Some(old));
        assert!(!backup.exists());

        old_copy(&backup, Scope::Account);
        fs::write(path.join("SKILL.md"), b"Local revision").unwrap();
        assert!(recover(&path, &backup, Scope::Account).is_err());
        assert!(installed(&backup).unwrap().is_some());
        assert_eq!(fs::read(path.join("SKILL.md")).unwrap(), b"Local revision");
    }

    #[cfg(unix)]
    #[test]
    fn preserves_unmanaged_destinations_and_symlinks() {
        use std::os::unix::fs::symlink;

        let home = tempfile::tempdir().unwrap();
        let path = skill_path(home.path(), ".agents");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), "Existing skill").unwrap();
        let receipt = receipt(Scope::Project);
        assert!(update(home.path(), ".agents", &path, &receipt, true).is_err());
        assert_eq!(
            fs::read_to_string(path.join("SKILL.md")).unwrap(),
            "Existing skill"
        );
        assert!(!path.join(RECEIPT).exists());

        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), home.path().join(".claude")).unwrap();
        let path = skill_path(home.path(), ".claude");
        assert!(update(home.path(), ".claude", &path, &receipt, true).is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);

        let path = skill_path(home.path(), ".cursor");
        old_copy(&path, Scope::Project);
        fs::rename(path.join("SKILL.md"), outside.path().join("SKILL.md")).unwrap();
        symlink(outside.path().join("SKILL.md"), path.join("SKILL.md")).unwrap();
        assert!(update(home.path(), ".cursor", &path, &receipt, true).is_err());
        assert!(path.join("SKILL.md").is_symlink());
    }
}
