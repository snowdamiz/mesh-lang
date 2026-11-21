use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub(crate) const DEFAULT_TIMEOUT_MS: u32 = 5_000;
pub(crate) const DIAGNOSTIC_LIMIT: u64 = 64 * 1024;

pub(crate) fn timeout_ms(default: u32) -> Result<u32, String> {
    let invalid = || "MESH_VERIFY_TIMEOUT_MS must be an unsigned integer".to_owned();
    match std::env::var("MESH_VERIFY_TIMEOUT_MS") {
        Ok(value) => value.parse().map_err(|_| invalid()),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => Err(invalid()),
    }
}

pub(crate) struct Output {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) enum RunError {
    Io(io::Error),
    Start(io::Error),
    Timeout,
}

pub(crate) fn run(
    command: &mut Command,
    input: &str,
    deadline: Instant,
) -> Result<Output, RunError> {
    run_with_limit(command, input, deadline, DIAGNOSTIC_LIMIT)
}

pub(crate) fn run_with_limit(
    command: &mut Command,
    input: &str,
    deadline: Instant,
    stdout_limit: u64,
) -> Result<Output, RunError> {
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or(RunError::Timeout)
    };
    remaining()?;
    let mut stdin = tempfile::tempfile().map_err(RunError::Io)?;
    stdin.write_all(input.as_bytes()).map_err(RunError::Io)?;
    stdin.rewind().map_err(RunError::Io)?;
    let stdout = tempfile::tempfile().map_err(RunError::Io)?;
    let stderr = tempfile::tempfile().map_err(RunError::Io)?;
    let duplicate = |file: &File| file.try_clone().map(Stdio::from).map_err(RunError::Io);
    // A checker cannot fill a pipe while the caller waits for its exit.
    let mut child = command
        .stdin(stdin)
        .stdout(duplicate(&stdout)?)
        .stderr(duplicate(&stderr)?)
        .spawn()
        .map_err(RunError::Start)?;

    let status = (|| loop {
        let delay = remaining()?.min(Duration::from_millis(5));
        match child.try_wait().map_err(RunError::Io)? {
            Some(status) => return Ok(status),
            None => std::thread::sleep(delay),
        }
    })();
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    Ok(Output {
        status,
        stdout: read_output(stdout, stdout_limit).map_err(RunError::Io)?,
        stderr: read_output(stderr, DIAGNOSTIC_LIMIT).map_err(RunError::Io)?,
    })
}

fn read_output(mut file: File, limit: u64) -> io::Result<String> {
    let mut output = String::new();
    file.rewind()?;
    file.take(limit + 1).read_to_string(&mut output)?;
    if output.len() as u64 > limit {
        return Err(io::Error::other(format!(
            "checker output exceeded {limit} bytes"
        )));
    }
    Ok(output)
}
