//! Run with `cargo bench -p mesh-rt --features fuzzing --bench collection_format`.
//! Setup, validation, and arena reset are outside the timed formatting operation.
use mesh_rt::{
    collections::{list, map, set},
    gc, string,
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Instant,
};

struct CountingAllocator;
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATED.fetch_add(size, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, size)
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn main() {
    gc::mesh_rt_init();
    println!("case,size,median_ns,min_ns,max_ns,allocated_bytes");
    for size in [16, 256, 2048] {
        let numbers = (0..size).map(|i| i.to_string()).collect::<Vec<_>>();
        for case in ["format_list", "format_map", "format_set"] {
            let expected = match case {
                "format_list" => format!("[{}]", numbers.join(", ")),
                "format_set" => format!("#{{{}}}", numbers.join(", ")),
                "format_map" => format!(
                    "%{{{}}}",
                    numbers
                        .iter()
                        .map(|n| format!("{n} => {n}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                _ => unreachable!(),
            };
            let mut samples = Vec::new();
            let mut allocated_bytes = 0;
            for sample in 0..12 {
                let source = match case {
                    "format_list" => {
                        let values: Vec<u64> = (0..size as u64).collect();
                        list::mesh_list_from_array(values.as_ptr(), size as i64)
                    }
                    "format_map" => (0..size as u64)
                        .fold(map::mesh_map_new(), |m, i| map::mesh_map_put(m, i, i)),
                    "format_set" => {
                        (0..size as u64).fold(set::mesh_set_new(), |s, i| set::mesh_set_add(s, i))
                    }
                    _ => unreachable!(),
                };
                let callback = string::mesh_int_to_string as *mut u8;
                ALLOCATED.store(0, Ordering::Relaxed);
                COUNT_ALLOCATIONS.store(sample == 0, Ordering::Relaxed);
                let start = Instant::now();
                let result = black_box(match case {
                    "format_list" => list::mesh_list_to_string(black_box(source), callback),
                    "format_map" => map::mesh_map_to_string(black_box(source), callback, callback),
                    "format_set" => set::mesh_set_to_string(black_box(source), callback),
                    _ => unreachable!(),
                });
                let elapsed = start.elapsed().as_nanos();
                COUNT_ALLOCATIONS.store(false, Ordering::Relaxed);
                if sample == 0 {
                    allocated_bytes = ALLOCATED.load(Ordering::Relaxed);
                }
                unsafe {
                    assert_eq!((*(result as *const string::MeshString)).as_str(), expected);
                }
                gc::mesh_rt_reset_for_fuzzing();
                if sample >= 3 {
                    samples.push(elapsed);
                }
            }
            samples.sort_unstable();
            println!(
                "{case},{size},{},{},{},{}",
                samples[4], samples[0], samples[8], allocated_bytes
            );
        }
    }
}
