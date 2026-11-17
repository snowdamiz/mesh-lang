mod actor;
mod http;
mod record;

use std::{
    env, error::Error, fs::File, hint::black_box, io::BufReader, num::NonZeroUsize,
    process::ExitCode, str::FromStr, time::Instant,
};

use serde::Serialize;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Serialize)]
struct Trial<Checksum> {
    ops: usize,
    elapsed_ns: u128,
    checksum: Checksum,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtt_ns: Option<Vec<u128>>,
}

fn env_value<T: FromStr>(name: &str) -> Result<Option<T>> {
    match env::var(name) {
        Ok(value) => value
            .parse()
            .map(Some)
            .map_err(|_| format!("invalid {name}: {value:?}").into()),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(format!("{name}: {error}").into()),
    }
}

fn required_env<T: FromStr>(name: &str) -> Result<T> {
    env_value(name)?.ok_or_else(|| format!("missing {name}").into())
}

fn cpu(iterations: usize, warmup: usize) -> Result<Trial<i64>> {
    let path: String = required_env("PERF_FIXTURES")?;
    let records: Vec<record::Record> = serde_json::from_reader(BufReader::new(File::open(path)?))?;
    if records.is_empty() && (iterations != 0 || warmup != 0) {
        return Err("PERF_FIXTURES must contain records for the requested iterations".into());
    }

    let checksum = |records: &[record::Record], count| {
        records
            .iter()
            .cycle()
            .take(count)
            .map(record::Record::score)
            .sum::<i64>()
    };
    if warmup != 0 {
        black_box(checksum(black_box(records.as_slice()), warmup));
    }

    let started = Instant::now();
    let checksum = checksum(&records, iterations);
    Ok(Trial {
        ops: iterations,
        elapsed_ns: started.elapsed().as_nanos(),
        checksum,
        rtt_ns: None,
    })
}

fn print_trial(trial: Trial<impl Serialize>) -> Result<()> {
    println!("{}", serde_json::to_string(&trial)?);
    Ok(())
}

fn run() -> Result<()> {
    let mode: String = required_env("PERF_MODE")?;
    if mode == "cpu" {
        return print_trial(cpu(
            required_env("PERF_ITERS")?,
            env_value("PERF_WARMUP")?.unwrap_or(0),
        )?);
    }

    let mut runtime = tokio::runtime::Builder::new_multi_thread();
    if let Some(workers) = env_value::<NonZeroUsize>("PERF_WORKERS")? {
        runtime.worker_threads(workers.get());
    }

    runtime.enable_io().enable_time().build()?.block_on(async {
        match mode.as_str() {
            "actor" => print_trial(
                actor::run(
                    required_env("PERF_PAIRS")?,
                    required_env("PERF_ITERS")?,
                    env_value("PERF_WARMUP")?.unwrap_or(0),
                )
                .await?,
            ),
            "recovery" => print_trial(actor::recovery().await?),
            "serve" => http::serve(required_env("PERF_PORT")?).await,
            _ => Err(format!("invalid PERF_MODE: {mode:?}").into()),
        }
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
