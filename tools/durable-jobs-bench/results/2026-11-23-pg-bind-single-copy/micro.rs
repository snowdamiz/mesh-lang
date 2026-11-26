use std::hint::black_box;
use std::io::{self, Write};
use std::time::Instant;

include!("extracted.rs");

#[cfg(profile_alloc)]
mod allocation {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

    static ENABLED: AtomicBool = AtomicBool::new(false);
    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);
    static ALLOCS: AtomicUsize = AtomicUsize::new(0);
    static REALLOCS: AtomicUsize = AtomicUsize::new(0);
    static MOVES: AtomicUsize = AtomicUsize::new(0);
    static MOVED_LAYOUT: AtomicUsize = AtomicUsize::new(0);
    static REQUESTED: AtomicUsize = AtomicUsize::new(0);

    #[global_allocator]
    static ALLOCATOR: Allocator = Allocator;

    struct Allocator;
    unsafe impl GlobalAlloc for Allocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let result = unsafe { System.alloc(layout) };
            if ENABLED.load(Relaxed) {
                ALLOCS.fetch_add(1, Relaxed);
                REQUESTED.fetch_add(layout.size(), Relaxed);
                PEAK.fetch_max(LIVE.fetch_add(layout.size(), Relaxed) + layout.size(), Relaxed);
            }
            result
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            if ENABLED.load(Relaxed) {
                LIVE.fetch_sub(layout.size(), Relaxed);
            }
            unsafe { System.dealloc(ptr, layout) };
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let result = unsafe { System.realloc(ptr, layout, size) };
            if ENABLED.load(Relaxed) {
                REALLOCS.fetch_add(1, Relaxed);
                REQUESTED.fetch_add(size, Relaxed);
                if ptr != result {
                    MOVES.fetch_add(1, Relaxed);
                    MOVED_LAYOUT.fetch_add(layout.size(), Relaxed);
                }
                let live = LIVE.fetch_sub(layout.size(), Relaxed) - layout.size() + size;
                LIVE.fetch_add(size, Relaxed);
                PEAK.fetch_max(live, Relaxed);
            }
            result
        }
    }

    pub fn start() {
        ENABLED.store(true, Relaxed);
    }

    pub fn report() {
        ENABLED.store(false, Relaxed);
        assert_eq!(LIVE.load(Relaxed), 0);
        for (name, value) in [
            ("allocs", &ALLOCS),
            ("reallocs", &REALLOCS),
            ("realloc_moves", &MOVES),
            ("moved_old_layout_bytes", &MOVED_LAYOUT),
            ("requested_bytes", &REQUESTED),
            ("peak_live_requested", &PEAK),
        ] {
            println!("{name}={}", value.load(Relaxed));
        }
    }
}

#[repr(C)]
struct Timespec {
    seconds: std::os::raw::c_long,
    nanoseconds: std::os::raw::c_long,
}

unsafe extern "C" {
    fn clock_gettime(id: i32, time: *mut Timespec) -> i32;
}

fn cpu_ns() -> u64 {
    #[cfg(target_os = "macos")]
    const PROCESS_CPU: i32 = 12;
    #[cfg(target_os = "linux")]
    const PROCESS_CPU: i32 = 2;
    let mut time = Timespec { seconds: 0, nanoseconds: 0 };
    assert_eq!(unsafe { clock_gettime(PROCESS_CPU, &mut time) }, 0);
    time.seconds as u64 * 1_000_000_000 + time.nanoseconds as u64
}

fn encode(
    optimized: bool,
    sql: Option<&str>,
    params: &[BindValue<'_>],
    formats: &[i16],
    describe: bool,
) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(sql) = sql {
        write_parse(&mut buf, sql);
    }
    let start = buf.len();
    let suffix = if describe { DESCRIBE_EXECUTE_SYNC_BYTES } else { EXECUTE_SYNC_BYTES };
    if optimized {
        optimized_bind(&mut buf, params, formats, suffix).unwrap();
    } else {
        stock_bind(&mut buf, params, formats).unwrap();
    }
    let end = buf.len();
    let capacity = buf.capacity();
    if describe {
        write_describe_portal(&mut buf);
    }
    write_execute(&mut buf);
    write_sync(&mut buf);
    assert_eq!(buf.len() - end, suffix);
    assert_eq!(buf.len() - (start + suffix), 1 + i32::from_be_bytes(buf[start + 1..start + 5].try_into().unwrap()) as usize);
    if optimized {
        assert_eq!(buf.capacity(), capacity);
    }
    buf
}

struct ShortWriter {
    bytes: Vec<u8>,
    remaining: usize,
    fail_flush: bool,
    writes: usize,
}

impl Write for ShortWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        self.writes += 1;
        let n = data.len().min(31).min(self.remaining);
        self.bytes.extend_from_slice(&data[..n]);
        self.remaining -= n;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        Ok(())
    }
}

struct PgConn {
    stream: ShortWriter,
    broken: bool,
}

fn check_wire(frame: &[u8]) {
    for (remaining, fail_flush, expected) in [
        (usize::MAX, false, frame.len()),
        (53, false, 53),
        (usize::MAX, true, frame.len()),
    ] {
        let mut conn = PgConn {
            stream: ShortWriter { bytes: Vec::new(), remaining, fail_flush, writes: 0 },
            broken: false,
        };
        let result = conn.write_wire_all(frame);
        assert_eq!(conn.stream.bytes, frame[..expected]);
        assert!(conn.stream.writes > 1);
        if remaining < frame.len() || fail_flush {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(conn.write_wire_all(frame).unwrap_err().kind(), io::ErrorKind::NotConnected);
            assert_eq!(conn.stream.bytes.len(), expected);
        } else {
            assert!(result.is_ok());
        }
    }
}

fn check() {
    let text = [0x61, 0x7a];
    let binary = [0, 0xff, 0x80];
    let mixed = [BindValue::Text(&text), BindValue::Binary(&binary), BindValue::Null];
    let typed = encode(true, None, &mixed, &[1, 0], true);
    assert_eq!(
        typed,
        [
            b'B', 0, 0, 0, 39, 0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0, 3,
            0, 0, 0, 2, b'a', b'z', 0, 0, 0, 3, 0, 0xff, 0x80, 0xff, 0xff,
            0xff, 0xff, 0, 2, 0, 1, 0, 0, b'D', 0, 0, 0, 6, b'P', 0,
            b'E', 0, 0, 0, 9, 0, 0, 0, 0, 0, b'S', 0, 0, 0, 4,
        ],
    );
    let chunk = vec![0x8a; MAX_DB_VALUE_BYTES + 1];
    let full = BindValue::Binary(&chunk[..MAX_DB_VALUE_BYTES]);
    let exact_remainder = MAX_PG_MESSAGE_BYTES - 12 - 4 * 6 - 3 * MAX_DB_VALUE_BYTES;
    let exact = [full, full, full, BindValue::Binary(&chunk[..exact_remainder])];
    let above = [full, full, full, BindValue::Binary(&chunk[..exact_remainder + 1])];
    let too_many = vec![BindValue::Null; MAX_PG_VALUES + 1];
    let max_count = vec![BindValue::Null; MAX_PG_VALUES];
    let overflow_32 = vec![full; 257];
    for params in [&[][..], &mixed, &max_count[..], &exact] {
        for (sql, formats, describe) in [(Some("SELECT $1"), &[][..], false), (Some("SELECT $1"), &[], true), (None, &[], true)] {
            assert_eq!(encode(false, sql, params, formats, describe), encode(true, sql, params, formats, describe));
        }
    }
    assert_eq!(typed, encode(false, None, &mixed, &[1, 0], true));
    for (params, expected) in [
        (&above[..], "PostgreSQL Bind message exceeds 67108864 byte limit"),
        (&[full, full, full, full, BindValue::Binary(&chunk)][..], "PostgreSQL parameter at index 4 exceeds 16777216 byte limit"),
        (&too_many[..], "too many PostgreSQL parameters: 32768 (maximum 32767)"),
        (&overflow_32[..], "PostgreSQL Bind message exceeds 67108864 byte limit"),
    ] {
        let mut prefix = Vec::with_capacity(17);
        prefix.extend_from_slice(b"PREFIX\0");
        for optimized in [false, true] {
            let cap = prefix.capacity();
            let result = if optimized {
                optimized_bind(&mut prefix, params, &[], DESCRIBE_EXECUTE_SYNC_BYTES)
            } else {
                stock_bind(&mut prefix, params, &[])
            };
            assert_eq!(result.unwrap_err(), expected);
            assert_eq!(prefix, b"PREFIX\0");
            assert_eq!(prefix.capacity(), cap);
        }
    }
    let complete = encode(true, Some("SELECT $1"), &mixed, &[], true);
    check_wire(&complete);
    println!("differential=ok fixed_vector=ok exact_limit=67108864 above_limit=ok max_count=32767 32_bit_sum=ok partial_writes=ok");
}

fn populated(count: usize, seed: u8) -> Vec<u8> {
    (0..count).map(|i| b'a' + ((i.wrapping_mul(2971) ^ (i >> 7) ^ seed as usize) % 26) as u8).collect()
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args[0] == "check" {
        check();
        return;
    }
    let optimized = args[0].starts_with("optimized");
    let job = std::path::Path::new(&args[3]);
    let job_values: Vec<_> = if args[1] == "jobs" {
        (0..4).map(|i| std::fs::read(job.with_extension(i.to_string())).unwrap()).collect()
    } else {
        Vec::new()
    };
    let (scalar_len, binary_len, iterations): (usize, usize, usize) = match args[1].as_str() {
        "short" => (32, 19, 1_000_000),
        "scalar" => (8 * 1024 * 1024, 0, 220),
        "mixed" => (2 * 1024 * 1024, 6 * 1024 * 1024, 220),
        "jobs" => (0, 0, 3000),
        shape => panic!("unknown shape: {shape}"),
    };
    let scalar = populated(scalar_len, 3);
    let binary: Vec<u8> = (0..binary_len).map(|i| (i.wrapping_mul(17) ^ (i >> 9)) as u8).collect();
    let (sql, params, formats, describe): (_, Vec<_>, &[_], _) = match args[1].as_str() {
        "mixed" => (None, vec![BindValue::Text(b"912"), BindValue::Binary(&binary), BindValue::Text(&scalar), BindValue::Null, BindValue::Text(b"")], &[1, 0, 0], true),
        "jobs" => (Some("SELECT mesh_jobs.enqueue($1, $2::jsonb, $3, $4)::text AS receipt"), job_values.iter().map(|bytes| BindValue::Text(bytes)).collect(), &[], true),
        _ => (Some("INSERT INTO bind_scalar VALUES ($1, $2)"), vec![BindValue::Text(b"912"), BindValue::Text(&scalar)], &[], false),
    };
    let baseline = encode(false, sql, &params, formats, describe);
    assert_eq!(encode(true, sql, &params, formats, describe), baseline);
    let frame_bytes = baseline.len();
    let bind_body = 8 + params.len() * 6 + formats.len() * 2
        + params.iter().map(|param| match param {
            BindValue::Text(bytes) | BindValue::Binary(bytes) => bytes.len(),
            BindValue::Null => 0,
        }).sum::<usize>();
    drop(baseline);
    let measured_iterations = if args[2] == "alloc" { 1 } else { iterations };
    for _ in 0..4 {
        black_box(encode(optimized, sql, &params, formats, describe));
    }
    let wall = Instant::now();
    let cpu = cpu_ns();
    #[cfg(profile_alloc)]
    allocation::start();
    for _ in 0..measured_iterations {
        let result = encode(black_box(optimized), sql, black_box(&params), formats, describe);
        black_box(result.as_slice());
    }
    #[cfg(profile_alloc)]
    allocation::report();
    println!("cpu_ns={}\nwall_ns={}\niterations={measured_iterations}\nframe_bytes={frame_bytes}\nexplicit_bind_body_copy_bytes={}", cpu_ns() - cpu, wall.elapsed().as_nanos(), if optimized { 0 } else { bind_body });
}
