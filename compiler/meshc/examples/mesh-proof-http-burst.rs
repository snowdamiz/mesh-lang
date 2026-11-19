#[path = "../src/proof/http.rs"]
mod http;

use std::io::{self, Write};
use std::net::ToSocketAddrs;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Instant;

use http::BurstResult;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let requests: usize = std::env::args()
        .nth(1)
        .ok_or("missing request count")?
        .parse()?;
    let gateways = [
        "gateway1:8080"
            .to_socket_addrs()?
            .next()
            .ok_or("missing gateway1")?,
        "gateway2:8080"
            .to_socket_addrs()?
            .next()
            .ok_or("missing gateway2")?,
    ];
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let workers = (0..requests)
        .map(|index| {
            let address = gateways[index % gateways.len()];
            let gate = Arc::clone(&gate);
            thread::Builder::new()
                .stack_size(256 * 1024)
                .spawn(move || {
                    let (opened, signal) = &*gate;
                    drop(
                        signal
                            .wait_while(opened.lock().unwrap(), |opened| !*opened)
                            .unwrap(),
                    );
                    let started = Instant::now();
                    let result = http::http_request(address, "GET", "/proof/pressure", "", &[]);
                    (
                        result,
                        started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = io::stdout().lock();
    writeln!(output, "ready")?;
    output.flush()?;
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    let results: Vec<BurstResult> = workers
        .into_iter()
        .map(|worker| worker.join().map_err(|_| "burst worker panicked"))
        .collect::<Result<_, _>>()?;
    serde_json::to_writer(output, &results)?;
    Ok(())
}
