//! Run with `cargo bench -p mesh-common --bench module_graph`.
//! Graph construction and correctness checks are outside the timed region.

use std::hint::black_box;
use std::time::{Duration, Instant};

use mesh_common::module_graph::{topological_sort, ModuleGraph, ModuleId};

fn graph(shape: &str, size: usize) -> ModuleGraph {
    let mut graph = ModuleGraph::new();
    for i in 0..size {
        graph.add_module(
            format!("Module{i:05}"),
            format!("module{i}.mpl").into(),
            false,
        );
        let dependencies = match shape {
            "chain" => i.saturating_sub(1)..i,
            "layered" => i.saturating_sub(8)..i,
            "independent" => 0..0,
            _ => unreachable!(),
        };
        for dependency in dependencies {
            graph.add_dependency(ModuleId(i as u32), ModuleId(dependency as u32));
        }
    }
    graph
}

fn time(graph: &ModuleGraph, iterations: usize) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(topological_sort(black_box(graph)).unwrap());
    }
    start.elapsed()
}

fn main() {
    println!("shape,modules,edges,iterations,median_ns,min_ns,max_ns");
    for shape in ["independent", "chain", "layered"] {
        for size in [32, 1024, 4096] {
            let graph = graph(shape, size);
            let expected: Vec<_> = (0..size).map(|i| ModuleId(i as u32)).collect();
            assert_eq!(topological_sort(&graph).unwrap(), expected);
            let edges: usize = graph.modules.iter().map(|m| m.dependencies.len()).sum();

            // Calibrate batches to reduce clock noise, then report seven samples.
            let mut iterations = 1;
            while time(&graph, iterations) < Duration::from_millis(20) {
                iterations *= 2;
            }
            let mut samples = [0.0; 7];
            for sample in &mut samples {
                *sample = time(&graph, iterations).as_nanos() as f64 / iterations as f64;
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "{shape},{size},{edges},{iterations},{:.0},{:.0},{:.0}",
                samples[3], samples[0], samples[6]
            );
        }
    }
}
