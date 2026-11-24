use super::{decode, test_context, Admission, Enqueue, JobSpec, MAX_BATCH, MAX_INPUT};
use sha2::{Digest, Sha256};
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

#[path = "enqueue_spec_wire.rs"]
mod wire;

#[derive(Clone, Copy, PartialEq)]
enum Arm {
    Stock,
    Candidate,
    Staged,
}

fn prepare(input: &str, arm: Arm, stage: &mut [u128; 5]) -> Result<(bool, Vec<Admission>), String> {
    let mark = (arm == Arm::Staged).then(Instant::now);
    let (batch, specs) = match decode(input)? {
        Enqueue::Single(spec) => (false, vec![*spec]),
        Enqueue::Batch(specs) => (true, specs),
    };
    if let Some(mark) = mark {
        stage[0] += mark.elapsed().as_nanos();
    }
    if specs.is_empty() || specs.len() > MAX_BATCH {
        return Err("mesh jobs enqueue requires 1 to 128 specs".into());
    }
    let admissions = match arm {
        Arm::Stock => specs
            .into_iter()
            .map(|mut spec| spec.prepare())
            .collect::<Result<Vec<_>, _>>()?,
        Arm::Candidate => specs
            .into_iter()
            .map(JobSpec::prepare_for_enqueue)
            .collect::<Result<Vec<_>, _>>()?,
        Arm::Staged => specs
            .into_iter()
            .map(|mut spec| spec.prepare_staged(stage))
            .collect::<Result<Vec<_>, _>>()?,
    };
    Ok((batch, admissions))
}

fn output(input: &str, arm: Arm) -> Result<(bool, Vec<Admission>), String> {
    prepare(input, arm, &mut [0; 5])
}

fn compare(input: &str) {
    match (output(input, Arm::Stock), output(input, Arm::Candidate)) {
        (Ok((a_batch, stock)), Ok((b_batch, candidate))) => {
            assert_eq!(a_batch, b_batch);
            assert_eq!(stock.len(), candidate.len());
            for (a, b) in stock.iter().zip(candidate.iter()) {
                assert!(a.spec == b.spec && a.request_key == b.request_key && a.fingerprint == b.fingerprint);
                #[cfg(probe_bulk)]
                assert_eq!(a.independent, b.independent);
                assert_eq!(format!("{:x}", Sha256::digest(b.spec.as_bytes())), b.fingerprint);
            }
        }
        (Err(a), Err(b)) => assert_eq!(a, b),
        _ => panic!("acceptance mismatch on a {} byte input", input.len()),
    }
}

fn corpus() -> usize {
    let cases = [
        (true, r#"{"worker":"worker"}"#),
        (true, r#"[{"worker":"worker"}]"#),
        (true, r#" {"worker":"α🙂","meta":{"a":{"b":4},"z":null},"args":{"dup":1,"dup":2,"é":"\u00e9","escaped":"\u001f\t\n\\\"","emoji":"\ud83d\ude42","m":[-0,-0.0,1.2e-26,3.14,18446744073709551615,-9223372036854775808,5e-324,1.7976931348623157e308]},"tags":["β","β"],"unique":{"fields":["args","meta","args"],"states":["completed","available"],"keys":["z","a","z"],"period_ms":"infinity","timestamp":"scheduled_at"},"replace":{"available":["meta","scheduled_at","args","meta"]},"request_key":"key","scheduled_at_ms":253402300799999}"#),
        (true, r#"{"worker":"worker","unique":true,"replace":["meta","worker","queue"]}"#),
        (true, r#"{"worker":"worker","unique":"key","replace":{"retryable":["tags"]}}"#),
        (true, r#"{"worker":"worker","unique":{"key":"key","fields":[],"period_ms":1}}"#),
        (false, r#"{"worker":"worker","unique":false,"replace":["args"]}"#),
        (false, r#"{"worker":"worker","unique":{"keys":[""],"states":[]}}"#),
        (false, r#"{"worker":"worker","unique":{"unexpected":true}}"#),
        (false, r#"{"worker":"worker","unique":{"period_ms":0}}"#),
        (true, r#"{"worker":"worker","args":{"zero":"\u0000"},"suspended":true,"delay_ms":5}"#),
        (false, r#"{"worker":"worker","meta":{},"metadata":{}}"#),
        (false, r#"{"worker":"worker","worker":"other"}"#),
        (false, r#"{"worker":"worker","me\u0074a":{},"meta":{}}"#),
        (false, r#"{"worker":"worker","unknown":true}"#),
        (false, r#"{"worker":"worker","args":{"broken":"\ud800"}}"#),
        (false, r#"{"worker":"worker","args":{"nan":1e9999}}"#),
        (false, r#"{"worker":"worker","args":[]}"#),
        (false, r#"{"worker":"worker","metadata":false}"#),
        (false, r#"{"worker":"worker","version":0}"#),
        (false, r#"{"worker":"worker","priority":10}"#),
        (false, r#"{"worker":"worker","max_attempts":101}"#),
        (false, r#"{"worker":"worker","max_attempts":2.0}"#),
        (false, r#"{"worker":"worker","timeout_ms":0}"#),
        (false, r#"{"worker":"worker","timeout_ms":31536000001}"#),
        (false, r#"{"worker":"worker","backoff_base_ms":2,"backoff_max_ms":1}"#),
        (false, r#"{"worker":"worker","delay_ms":1,"scheduled_at_ms":1}"#),
        (false, r#"{"worker":"worker","delay_ms":31536000001}"#),
        (false, r#"{"worker":"worker","request_key":""}"#),
        (false, r#"{"worker":"worker","tags":["\u0000"]}"#),
        (true, r#"{"worker":"worker","request_key":null}"#),
        (false, r#"[{"worker":"good"},{"worker":""}]"#),
        (false, r#"[{"worker":"good"},{"unknown":0}]"#),
        (true, r#"["worker",1,{},"default",{},0,[],null,null,20,60000,1000,3600000,false,null,null,null]"#),
        (false, "[]"), (false, "{}"), (false, "null"), (false, "true"), (false, "42"),
        (false, "["), (false, "{}{}"), (false, "\u{feff}{}"),
    ];
    for (valid, input) in cases {
        assert_eq!(output(input, Arm::Stock).is_ok(), valid, "{input}");
        compare(input);
    }
    for jobs in [127, 128, 129] {
        compare(&format!("[{}]", vec![cases[0].1; jobs].join(",")));
    }
    for tags in [127, 128, 129] {
        compare(&format!(r#"{{"worker":"x","tags":[{}]}}"#, vec![r#""x""#; tags].join(",")));
    }
    for len in [MAX_INPUT - 1, MAX_INPUT, MAX_INPUT + 1] {
        compare(&format!("{}{}", cases[0].1, " ".repeat(len - cases[0].1.len())));
    }
    for depth in [0, 1, 120, 126, 127, 128, 129] {
        compare(&format!(
            r#"{{"worker":"x","args":{{"nested":{}0{}}}}}"#,
            "[".repeat(depth),
            "]".repeat(depth)
        ));
    }
    let context: serde_json::Value = serde_json::from_str(&test_context(
        r#"{"worker":"worker","args":{"x":"é"},"meta":{"t":7},"tags":["a"],"delay_ms":5}"#,
        30,
    ).unwrap()).unwrap();
    assert_eq!(context["args"]["x"], "é");
    assert_eq!(context["metadata"]["t"], 7);
    assert_eq!(context["tags"][0], "a");
    assert_eq!(context["scheduled_at_ms"], 35);
    let id = context["id"].as_str().unwrap();
    assert_eq!(context["effect_key"], format!("{id}/1"));
    cases.len() + 17
}

fn frames(path: &Path) -> Vec<String> {
    let count = std::fs::read_dir(path).unwrap().count();
    (0..count).map(|i| std::fs::read_to_string(path.join(i.to_string())).unwrap()).collect()
}

fn output_digest(frames: &[String], arm: Arm) -> (usize, String) {
    let mut hash = Sha256::new();
    let mut jobs = 0;
    for input in frames {
        let (batch, records) = output(input, arm).unwrap();
        hash.update([u8::from(batch)]);
        hash.update((records.len() as u64).to_be_bytes());
        jobs += records.len();
        for a in records {
            for field in [a.spec, a.request_key, a.fingerprint] {
                hash.update((field.len() as u64).to_be_bytes());
                hash.update(field.as_bytes());
            }
        }
    }
    (jobs, format!("{:x}", hash.finalize()))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn time_allowed() {
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    assert_eq!(boot.trim(), "90ec1c24-08c8-4bc1-aa28-79b6db1bd13a");
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    assert!(status.lines().any(|line| line.strip_prefix("Cpus_allowed_list:").is_some_and(|s| s.trim() == "0")));
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn time_allowed() {
    panic!("timing requires the pinned Linux AMD64 Linode");
}

fn cpu_ns() -> u128 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    assert_eq!(unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) }, 0);
    ts.tv_sec as u128 * 1_000_000_000 + ts.tv_nsec as u128
}

pub(crate) fn run() {
    let args: Vec<_> = std::env::args().collect();
    if args[1] == "verify-wire" {
        wire::run();
        return;
    }
    let frames = frames(Path::new(&args[2]));
    if args[1] == "verify" {
        let cases = corpus();
        for frame in &frames {
            compare(frame);
        }
        let (jobs, sha) = output_digest(&frames, Arm::Stock);
        assert_eq!((jobs, sha.clone()), output_digest(&frames, Arm::Candidate));
        println!("{{\"cases\":{cases},\"frames\":{},\"jobs\":{jobs},\"output_sha256\":\"{sha}\"}}", frames.len());
        return;
    }
    time_allowed();
    let arm = match args[1].as_str() {
        "stock" => Arm::Stock,
        "candidate" => Arm::Candidate,
        "stage" => Arm::Staged,
        _ => panic!("unknown arm"),
    };
    let repeats = args[3].parse::<usize>().unwrap();
    assert!((1..=256).contains(&repeats));
    let mut stage = [0; 5];
    let cpu = cpu_ns();
    let wall = Instant::now();
    let mut count = 0;
    for _ in 0..repeats {
        for input in &frames {
            let (_, records) = prepare(black_box(input), arm, &mut stage).unwrap();
            count += records.len();
            drop(black_box(records));
        }
    }
    let wall_ns = wall.elapsed().as_nanos();
    let cpu_ns = cpu_ns() - cpu;
    let (jobs, sha) = output_digest(&frames, arm);
    assert_eq!(count, jobs * repeats);
    println!(
        "{{\"frames\":{},\"jobs\":{jobs},\"repeats\":{repeats},\"accepted\":{count},\"cpu_ns\":{cpu_ns},\"wall_ns\":{wall_ns},\"stage_wall_ns\":{stage:?},\"output_sha256\":\"{sha}\"}}",
        frames.len()
    );
}
