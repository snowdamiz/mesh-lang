//! Run with `cargo bench -p mesh-rt --bench routing`.
//! Construction and correctness checks are outside the timed region.

use mesh_rt::dist::{
    routing::{select_record_replicas, NodeLoadReport},
    telemetry::{NodeLifecycleState, NodeRoles},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::BTreeSet,
    hint::black_box,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{Duration, Instant},
};

struct CountingAllocator;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(ptr, layout, size)
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn candidates(size: usize) -> Vec<NodeLoadReport> {
    (0..size)
        .map(|index| {
            let node = index * 73 % size;
            NodeLoadReport {
                protocol_version: 3,
                node_id: format!("worker-{node:04}"),
                boot_id: format!("boot-{node}"),
                roles: NodeRoles::new(false, false, true),
                state: NodeLifecycleState::Ready,
                capacity_units: 1,
                active_workers: 2,
                runnable_actors: 0,
                inflight: 0,
                queued_items: 0,
                queued_bytes: 0,
                outstanding_reservations: 0,
                p95_queue_wait: Duration::ZERO,
                memory_pressure: 0.0,
                decision_pressure_ewma: 0.0,
                sequence: 1,
                control_term: 1,
                membership_generation: 1,
                failure_domain: format!("region-{}", node % 8),
                handlers: BTreeSet::new(),
            }
        })
        .collect()
}

fn time(candidates: &[NodeLoadReport], iterations: usize) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(select_record_replicas("worker-0000", 3, black_box(candidates)).unwrap());
    }
    start.elapsed()
}

fn main() {
    println!("case,candidates,iterations,median_ns,min_ns,max_ns,allocations");
    for size in [8, 32, 128, 512] {
        let candidates = candidates(size);
        let expected = if size == 8 {
            ["worker-0001", "worker-0002"]
        } else {
            ["worker-0001", "worker-0009"]
        };
        assert_eq!(
            select_record_replicas("worker-0000", 3, &candidates).unwrap(),
            expected
        );

        // Count separately from timing, after one call initializes shared state.
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        COUNT_ALLOCATIONS.store(true, Ordering::Relaxed);
        let result = select_record_replicas("worker-0000", 3, &candidates).unwrap();
        COUNT_ALLOCATIONS.store(false, Ordering::Relaxed);
        let allocations = ALLOCATIONS.load(Ordering::Relaxed) - before;
        assert_eq!(result, expected);

        let mut iterations = 1;
        while time(&candidates, iterations) < Duration::from_millis(20) {
            iterations *= 2;
        }
        let mut samples = [0.0; 7];
        for sample in &mut samples {
            *sample = time(&candidates, iterations).as_nanos() as f64 / iterations as f64;
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "replicas,{size},{iterations},{:.0},{:.0},{:.0},{allocations}",
            samples[3], samples[0], samples[6]
        );
    }
}
