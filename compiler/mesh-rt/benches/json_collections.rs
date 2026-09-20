//! Run with `cargo bench -p mesh-rt --features fuzzing --bench json_collections`.
//! The existing fuzz arena reset bounds memory; no Mesh pointers survive a sample.
use mesh_rt::{
    collections::{list, map},
    gc, json, string,
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

extern "C" fn int_to_json(value: u64) -> *mut u8 {
    json::mesh_json_from_int(value as i64)
}

fn main() {
    gc::mesh_rt_init();
    println!("case,size,median_ns,min_ns,max_ns,system_allocated_bytes");
    for size in [16, 256, 2048] {
        let array = serde_json::to_string(&(0..size).collect::<Vec<_>>()).unwrap();
        let object = serde_json::to_string(
            &(0..size)
                .map(|i| (format!("key_{i:05}"), serde_json::Value::from(i)))
                .collect::<serde_json::Map<_, _>>(),
        )
        .unwrap();
        for case in [
            "parse_array",
            "parse_object",
            "encode_object",
            "map_keys",
            "map_values",
            "from_list",
            "to_list",
        ] {
            let mut samples = Vec::new();
            let mut allocated_bytes = 0;
            for sample in 0..12 {
                let text = if matches!(case, "parse_array" | "from_list" | "to_list") {
                    &array
                } else {
                    &object
                };
                let input = string::mesh_string_new(text.as_ptr(), text.len() as u64);
                let parsed = if case.starts_with("parse_") {
                    std::ptr::null_mut()
                } else {
                    json::mesh_json_parse_raw(input)
                };
                let source = if case == "from_list" {
                    let values: Vec<u64> = (0..size as u64).collect();
                    list::mesh_list_from_array(values.as_ptr(), size as i64)
                } else if case.starts_with("map_") {
                    unsafe { (*(parsed as *const json::MeshJson)).value as *mut u8 }
                } else {
                    parsed
                };
                let before = ALLOCATED.load(Ordering::Relaxed);
                COUNT_ALLOCATIONS.store(sample == 0, Ordering::Relaxed);
                let start = Instant::now();
                let result = black_box(match case {
                    "parse_array" | "parse_object" => {
                        json::mesh_json_parse(black_box(input)).cast()
                    }
                    "encode_object" => json::mesh_json_encode(black_box(source)).cast(),
                    "map_keys" => map::mesh_map_keys(black_box(source)),
                    "map_values" => map::mesh_map_values(black_box(source)),
                    "from_list" => json::mesh_json_from_list(black_box(source), int_to_json),
                    "to_list" => json::mesh_json_to_list(black_box(source), json::mesh_json_as_int),
                    _ => unreachable!(),
                });
                let elapsed = start.elapsed().as_nanos();
                COUNT_ALLOCATIONS.store(false, Ordering::Relaxed);
                if sample == 0 {
                    allocated_bytes = ALLOCATED.load(Ordering::Relaxed) - before;
                }
                unsafe {
                    match case {
                        "parse_array" | "parse_object" | "to_list" => {
                            let result = &*(result as *const mesh_rt::io::MeshResult);
                            assert_eq!(result.tag, 0);
                            if case == "to_list" {
                                assert_eq!(list::mesh_list_length(result.value), size as i64);
                                for i in 0..size {
                                    assert_eq!(
                                        list::mesh_list_get(result.value, i as i64),
                                        i as u64
                                    );
                                }
                            } else {
                                assert_eq!(
                                    serde_json::from_str::<serde_json::Value>(
                                        (*json::mesh_json_encode(result.value)).as_str()
                                    )
                                    .unwrap(),
                                    serde_json::from_str::<serde_json::Value>(text).unwrap()
                                );
                            }
                        }
                        "encode_object" => {
                            assert_eq!((*(result as *const string::MeshString)).as_str(), object)
                        }
                        "map_keys" | "map_values" => {
                            assert_eq!(list::mesh_list_length(result), size as i64);
                            for i in 0..size {
                                let value = list::mesh_list_get(result, i as i64);
                                if case == "map_keys" {
                                    assert_eq!(
                                        (*(value as *const string::MeshString)).as_str(),
                                        format!("key_{i:05}")
                                    );
                                } else {
                                    assert_eq!((*(value as *const json::MeshJson)).value, i as u64);
                                }
                            }
                        }
                        "from_list" => {
                            assert_eq!((*json::mesh_json_encode(result)).as_str(), array)
                        }
                        _ => unreachable!(),
                    }
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
