use super::{eventually, Worker};
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;

pub(super) struct AttemptProcess {
    pid: u32,
    start_ticks: u64,
}

impl AttemptProcess {
    pub(super) fn observe(runner: &Worker) -> Self {
        let runner_pid = runner.child.as_ref().unwrap().id();
        eventually(
            "the active attempt to have a Linux process identity",
            || {
                let mut children = BTreeSet::new();
                // The child belongs to the spawning thread, which may vary by runner.
                for thread in fs::read_dir(format!("/proc/{runner_pid}/task")).unwrap() {
                    let path = thread.unwrap().path().join("children");
                    match fs::read_to_string(path) {
                        Ok(pids) => children.extend(
                            pids.split_whitespace()
                                .map(|pid| pid.parse::<u32>().unwrap()),
                        ),
                        Err(error) if error.kind() == ErrorKind::NotFound => continue,
                        Err(error) => panic!("read runner children: {error}"),
                    }
                }
                let pid = *children.first()?;
                assert_eq!(children.len(), 1, "the runner must have one active attempt");
                let (state, start_ticks) = status(pid)?;
                assert!(!matches!(state, 'Z' | 'X'), "attempt {pid} already exited");
                Some(Self { pid, start_ticks })
            },
        )
    }

    pub(super) fn assert_exited(self) {
        eventually(
            &format!("Linux attempt process {} to exit", self.pid),
            || {
                status(self.pid)
                    .is_none_or(|(state, start)| {
                        start != self.start_ticks || matches!(state, 'Z' | 'X')
                    })
                    .then_some(())
            },
        );
    }
}

fn status(pid: u32) -> Option<(char, u64)> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        Err(error) => panic!("read attempt {pid} status: {error}"),
    };
    let (_, stats) = stat.rsplit_once(')').expect("Linux process name");
    let mut fields = stats.split_whitespace();
    let state = fields
        .next()
        .expect("Linux child state")
        .chars()
        .next()
        .unwrap();
    let start_ticks = fields
        .nth(18)
        .expect("Linux child start time")
        .parse()
        .unwrap();
    Some((state, start_ticks))
}
