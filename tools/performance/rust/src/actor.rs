use std::{sync::Arc, time::Instant};

use tokio::{
    sync::{Barrier, mpsc, watch},
    task::JoinHandle,
};

use crate::{Result, Trial};

const SAMPLE_INTERVAL: usize = 64;

enum Command {
    Add(u64),
    Fail,
}

#[derive(PartialEq, Eq)]
enum Exit {
    Closed,
    Failed,
}

struct Worker {
    requests: mpsc::Sender<Command>,
    replies: mpsc::Receiver<u64>,
}

impl Worker {
    fn spawn(ready: Option<Arc<Barrier>>) -> (Self, JoinHandle<Exit>) {
        let (requests, mut commands) = mpsc::channel(1);
        let (responses, replies) = mpsc::channel(1);

        let task = tokio::spawn(async move {
            let mut counter = 0;
            if let Some(ready) = ready {
                ready.wait().await;
            }
            while let Some(command) = commands.recv().await {
                match command {
                    Command::Add(value) => {
                        counter += value;
                        if responses.send(counter).await.is_err() {
                            break;
                        }
                    }
                    Command::Fail => return Exit::Failed,
                }
            }
            Exit::Closed
        });

        (Self { requests, replies }, task)
    }

    async fn add(&mut self, value: u64) -> Result<u64> {
        self.requests.send(Command::Add(value)).await?;
        self.replies
            .recv()
            .await
            .ok_or_else(|| "worker stopped before replying".into())
    }
}

struct CallerResult {
    counter: u64,
    finished_at: Option<Instant>,
    rtt_ns: Vec<u128>,
}

async fn call(
    mut worker: Worker,
    iterations: usize,
    measure: bool,
    mut rtt_ns: Vec<u128>,
) -> Result<CallerResult> {
    let mut counter = 0;
    let mut finished_at = None;
    for iteration in 1..=iterations {
        let offset = (iteration - 1) % SAMPLE_INTERVAL;
        let phase = ((iteration - 1) / SAMPLE_INTERVAL * 17) % SAMPLE_INTERVAL;
        let finished = measure && iteration == iterations;
        let sample =
            (measure && (offset == phase || (finished && offset < phase))).then(Instant::now);
        let reply = worker.add(iteration as u64).await?;
        if finished || sample.is_some() {
            let received_at = Instant::now();
            if let Some(started) = sample {
                rtt_ns.push((received_at - started).as_nanos());
            }
            if finished {
                finished_at = Some(received_at);
            }
        }
        counter += iteration as u64;
        if reply != counter {
            return Err(
                format!("iteration {iteration}: expected counter {counter}, got {reply}").into(),
            );
        }
    }
    Ok(CallerResult {
        counter,
        finished_at,
        rtt_ns,
    })
}

async fn phase(pairs: usize, iterations: usize, measure: bool) -> Result<Trial<u128>> {
    let ops = pairs
        .checked_mul(iterations)
        .ok_or("actor operation count overflow")?;
    let steps = iterations as u128;
    if steps * (steps + 1) / 2 > u128::from(u64::MAX) {
        return Err("actor counter overflow".into());
    }

    let participants = pairs
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .ok_or("actor task count overflow")?;
    let ready = Arc::new(Barrier::new(participants));
    let (start, starting) = watch::channel(());
    let mut tasks = Vec::with_capacity(pairs);
    let mut results = Vec::with_capacity(pairs);

    for _ in 0..pairs {
        let (worker, worker_task) = Worker::spawn(Some(ready.clone()));
        let (ready, mut starting) = (ready.clone(), starting.clone());
        let caller_task = tokio::spawn(async move {
            let samples = if measure {
                Vec::with_capacity(iterations.div_ceil(SAMPLE_INTERVAL))
            } else {
                Vec::new()
            };
            ready.wait().await;
            starting.changed().await?;
            call(worker, iterations, measure, samples).await
        });
        tasks.push((caller_task, worker_task));
    }

    ready.wait().await;
    let started = measure.then(Instant::now);
    start.send_replace(());

    for (caller_task, _) in &mut tasks {
        results.push(caller_task.await??);
    }

    let mut checksum = 0;
    let mut elapsed_ns = 0;
    let mut rtt_ns = Vec::new();
    for (result, (_, worker_task)) in results.into_iter().zip(tasks) {
        if worker_task.await? != Exit::Closed {
            return Err("actor worker failed".into());
        }
        checksum += u128::from(result.counter);
        if let (Some(started), Some(finished)) = (started, result.finished_at) {
            elapsed_ns = elapsed_ns.max((finished - started).as_nanos());
        }
        rtt_ns.extend(result.rtt_ns);
    }
    Ok(Trial {
        ops,
        elapsed_ns,
        checksum,
        rtt_ns: measure.then_some(rtt_ns),
    })
}

pub(crate) async fn run(pairs: usize, iterations: usize, warmup: usize) -> Result<Trial<u128>> {
    if warmup != 0 {
        phase(pairs, warmup, false).await?;
    }
    phase(pairs, iterations, true).await
}

async fn supervise(generations: mpsc::Sender<Worker>) -> Result<()> {
    loop {
        let (worker, task) = Worker::spawn(None);
        if generations.send(worker).await.is_err() || task.await? == Exit::Closed {
            return Ok(());
        }
    }
}

pub(crate) async fn recovery() -> Result<Trial<u64>> {
    let (generations, mut replacements) = mpsc::channel(1);
    let supervisor = tokio::spawn(supervise(generations));
    let mut primary = replacements.recv().await.ok_or("missing primary worker")?;
    let (mut sibling, sibling_task) = Worker::spawn(None);

    if primary.add(7).await? != 7 || sibling.add(11).await? != 11 {
        return Err("unexpected recovery worker state before failure".into());
    }

    let started = Instant::now();
    primary.requests.send(Command::Fail).await?;
    let mut replacement = replacements
        .recv()
        .await
        .ok_or("missing replacement worker")?;
    let reply = replacement.add(1).await?;
    let elapsed_ns = started.elapsed().as_nanos();

    if reply != 1 || sibling.add(1).await? != 12 {
        return Err("unexpected recovery worker state after failure".into());
    }
    drop(replacement);
    drop(sibling);
    supervisor.await??;
    sibling_task.await?;

    Ok(Trial {
        ops: 1,
        elapsed_ns,
        checksum: 1,
        rtt_ns: None,
    })
}
