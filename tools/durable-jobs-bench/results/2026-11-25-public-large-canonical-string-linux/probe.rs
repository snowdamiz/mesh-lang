use super::{decode, prepare_enqueue, test_context, Enqueue, JobSpec, MAX_BATCH, MAX_INPUT};
use crate::db::{jobs_connection, pg, pool};
use crate::io::MeshResult;
use crate::string::{mesh_str, text_of};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::hint::black_box;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

fn record(input: &str, out: &mut impl Write) -> std::io::Result<()> {
    match prepare_enqueue(input) {
        Ok((batch, records)) => {
            out.write_all(&[u8::from(batch)])?;
            out.write_all(&(records.len() as u64).to_be_bytes())?;
            for admission in records {
                assert_eq!(format!("{:x}", Sha256::digest(admission.spec.as_bytes())),
                           admission.fingerprint);
                for field in [admission.spec, admission.request_key, admission.fingerprint] {
                    field_bytes(field.as_bytes(), out)?;
                }
            }
        }
        Err(error) => {
            out.write_all(&[2])?;
            field_bytes(error.as_bytes(), out)?;
        }
    }
    Ok(())
}

fn field_bytes(bytes: &[u8], out: &mut impl Write) -> std::io::Result<()> {
    out.write_all(&(bytes.len() as u64).to_be_bytes())?;
    out.write_all(bytes)
}

fn output_digest(frames: &[String]) -> (usize, String) {
    let mut hash = Sha256::new();
    let mut jobs = 0;
    for input in frames {
        let (batch, records) = prepare_enqueue(input).unwrap();
        hash.update([u8::from(batch)]);
        hash.update((records.len() as u64).to_be_bytes());
        jobs += records.len();
        for admission in records {
            for field in [admission.spec, admission.request_key, admission.fingerprint] {
                hash.update((field.len() as u64).to_be_bytes());
                hash.update(field.as_bytes());
            }
        }
    }
    (jobs, format!("{:x}", hash.finalize()))
}

fn extended(emit: &mut impl FnMut(&str)) -> usize {
    let string = "a".repeat(65_536);
    let mut cases = 0;
    let mut valid = |input: String, eligible| {
        let spec = match decode::<Enqueue>(&input).unwrap() {
            Enqueue::Single(spec) => spec,
            Enqueue::Batch(mut batch) => Box::new(batch.remove(0)),
        };
        // SELECTED
        assert!(prepare_enqueue(&input).is_ok());
        emit(&input);
        cases += 1;
    };
    for len in [65_535, 65_536, 65_537] {
        valid(json!({"worker":"deliver","args":{"x":"a".repeat(len)}}).to_string(),
              len >= 65_536);
    }
    for over in [0, 1, 2] {
        let mut input = json!({"worker":"deliver","args":{"x":&string}}).to_string();
        input.push_str(&" ".repeat(2 * string.len() + over - input.len()));
        valid(input, over == 0);
    }
    for offset in [0, 127, 128, 65_535] {
        for escaped in (0u8..=31).map(char::from).chain(['"', '\\']) {
            let mut text = string.clone();
            text.replace_range(offset..offset + 1, &escaped.to_string());
            valid(json!({"worker":"deliver","args":{"x":text}}).to_string(), false);
        }
    }
    let unicode = "é🙂".repeat(21_846);
    valid(json!({"worker":"deliver","args":{"a":65,"ø🔐":&unicode,"z":{"x":"\\\u{1f}"}}}).to_string(), true);
    valid(json!({"worker":"deliver","args":{"x":"a".repeat(400_000),"z":"\n".repeat(65_536)}}).to_string(), true);
    valid(json!({"worker":"deliver","args":{"nested":{"x":&string}}}).to_string(), false);
    valid(json!([{"worker":"deliver","args":{"x":&string}},{"worker":"deliver"}]).to_string(), true);
    valid(json!([{"worker":"deliver","args":{"x":&string}},{"worker":"deliver","args":{"x":&string}}]).to_string(), false);
    valid(json!(["deliver",1,{"x":&string},"default",{},0,[],null,null,20,60000,1000,3600000,false,"positional",null,null]).to_string(), true);
    valid(format!(r#"{{"worker":"deliver","args":{{"x":"old","x":"{string}"}}}}"#), true);
    for invalid in [
        format!(r#"{{"worker":"","args":{{"x":"{string}"}}}}"#),
        format!(r#"{{"worker":"deliver","args":{{"x":"{string}"}},"unique":false,"replace":["args"]}}"#),
        format!(r#"{{"worker":"deliver","args":{{"x":"{string}\ud800"}}}}"#),
    ] {
        assert!(prepare_enqueue(&invalid).is_err());
        emit(&invalid);
        cases += 1;
    }
    cases
}

fn verify(path: &Path, snapshot: &Path, include_corpus: bool) {
    let frames = frames(path);
    let mut snapshot = BufWriter::new(std::fs::File::create(snapshot).unwrap());
    let cases = if include_corpus {
        let mut emit = |input: &str| {
            record(input, &mut snapshot).unwrap();
            let independent = prepare_enqueue(input).map(|(_, values)| {
                values.iter().map(|value| value.independent).collect::<Vec<_>>()
            });
            field_bytes(format!("{independent:?}").as_bytes(), &mut snapshot).unwrap();
        };
        let count = corpus(&mut emit) + extended(&mut emit);
        for invalid in [b"\xff".as_slice(), b"\xc0\x80", b"\xed\xa0\x80", b"\xf0\x9f\x99"] {
            assert!(std::str::from_utf8(invalid).is_err());
        }
        assert!(std::str::from_utf8("é🙂".as_bytes()).is_ok());
        count
    } else {
        0
    };
    for input in &frames {
        let Enqueue::Batch(mut specs) = decode(input).unwrap() else { panic!("expected FILE batch") };
        assert_eq!(specs.len(), 1);
        let spec = specs.pop().unwrap();
        let eligible = include_corpus;
        // SELECTED
        record(input, &mut snapshot).unwrap();
    }
    snapshot.flush().unwrap();
    let (jobs, output_sha256) = output_digest(&frames);
    println!("{}", json!({"cases":cases,"frames":frames.len(),"jobs":jobs,"output_sha256":output_sha256}));
}

fn wire(args: &[String]) {
    crate::gc::mesh_rt_init();
    let url = std::env::var("ENQUEUE_SPEC_TEST_PG_URL").unwrap();
    let namespace = mesh_str(&args[3]);
    let raw = if args[2] == "migrate" {
        None
    } else {
        let result = unsafe { &*crate::file::mesh_file_read(mesh_str(&args[4])) };
        if result.tag == 1 {
            let error = unsafe { text_of(result.value) };
            assert!(error.contains("valid UTF-8"), "{error}");
            println!("{}", json!({"invalid_utf8":true}));
            return;
        }
        assert_eq!(result.tag, 0);
        Some(unsafe { text_of(result.value) }.to_owned())
    };
    let handle = unsafe {
        let result = &*pool::mesh_pool_open(mesh_str(&url), 0, 1, 3_000).cast::<MeshResult>();
        assert_eq!(result.tag, 0);
        pool::unbox_u64_payload(result.value)
    };
    let result = match args[2].as_str() {
        "migrate" => super::super::mesh_jobs_migrate(handle),
        "enqueue" => super::super::mesh_jobs_enqueue(handle, namespace, mesh_str(raw.as_ref().unwrap())),
        "outer" => {
            let conn = unsafe {
                let result = &*pg::mesh_pg_connect(mesh_str(&url)).cast::<MeshResult>();
                assert_eq!(result.tag, 0);
                pool::unbox_u64_payload(result.value)
            };
            unsafe { jobs_connection::with_connection(conn, |db| db.simple("BEGIN")).unwrap() };
            let result = super::super::mesh_jobs_enqueue_in(conn, namespace, mesh_str(raw.as_ref().unwrap()));
            unsafe { jobs_connection::with_connection(conn, |db| {
                assert_eq!(super::response(db, "SELECT 'outer remains open' AS value", &[])?, "outer remains open");
                db.simple("ROLLBACK")
            }).unwrap() };
            pg::mesh_pg_close(conn);
            result
        }
        _ => panic!("unknown wire mode"),
    };
    let receipt = unsafe {
        let result = &*result;
        if result.tag == 0 && args[2] == "migrate" {
            json!({"ok":true})
        } else if result.tag == 0 {
            json!({"ok":serde_json::from_str::<serde_json::Value>(text_of(result.value)).unwrap()})
        } else {
            assert_eq!(result.tag, 1);
            json!({"error": text_of(result.value)})
        }
    };
    pool::mesh_pool_close(handle);
    println!("{receipt}");
}

pub(crate) fn run() {
    let args: Vec<_> = std::env::args().collect();
    if args[1] == "wire" {
        wire(&args);
        return;
    }
    let path = Path::new(&args[2]);
    if args[1] == "verify" {
        verify(path, Path::new(&args[3]), args[4] == "plain");
        return;
    }
    assert_eq!(args[1], "time");
    time_allowed();
    let frames = frames(path);
    let repeats = args[3].parse::<usize>().unwrap();
    assert!((1..=256).contains(&repeats));
    let cpu = cpu_ns();
    let wall = Instant::now();
    let mut count = 0;
    for _ in 0..repeats {
        for input in &frames {
            let (_, records) = prepare_enqueue(black_box(input)).unwrap();
            count += records.len();
            drop(black_box(records));
        }
    }
    let wall_ns = wall.elapsed().as_nanos();
    let cpu_ns = cpu_ns() - cpu;
    let (jobs, output_sha256) = output_digest(&frames);
    assert_eq!(count, jobs * repeats);
    println!("{}", json!({"frames":frames.len(),"jobs":jobs,"repeats":repeats,"accepted":count,
                          "cpu_ns":cpu_ns,"wall_ns":wall_ns,"output_sha256":output_sha256}));
}
