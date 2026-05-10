use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use tempfile::TempDir;

#[derive(Debug)]
struct BenchmarkSummary {
    sequential_runs: Vec<Duration>,
    parallel_runs: Vec<Duration>,
}

fn collect_json_paths(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .expect("failed to read benchmark theme directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.extension().and_then(|extension| extension.to_str()) == Some("json")
        })
        .collect();
    paths.sort();
    paths
}

fn sequential_read_and_parse(paths: &[PathBuf]) -> usize {
    let mut parsed_count = 0usize;
    for path in paths {
        if let Ok(content) = fs::read_to_string(path) {
            if serde_json::from_str::<Value>(&content).is_ok() {
                parsed_count += 1;
            }
        }
    }
    parsed_count
}

fn parallel_read_then_parse(paths: &[PathBuf]) -> usize {
    if paths.is_empty() {
        return 0;
    }

    let workers = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .min(paths.len());
    let chunk_size = paths.len().div_ceil(workers);

    let mut loaded = Vec::<String>::with_capacity(paths.len());
    thread::scope(|scope| {
        let mut handles = Vec::new();

        for chunk in paths.chunks(chunk_size) {
            let chunk = chunk.to_vec();
            handles.push(scope.spawn(move || {
                let mut out = Vec::with_capacity(chunk.len());
                for path in chunk {
                    if let Ok(content) = fs::read_to_string(path) {
                        out.push(content);
                    }
                }
                out
            }));
        }

        for handle in handles {
            if let Ok(mut part) = handle.join() {
                loaded.append(&mut part);
            }
        }
    });

    loaded
        .into_iter()
        .filter(|content| serde_json::from_str::<Value>(content).is_ok())
        .count()
}

fn setup_benchmark_themes() -> TempDir {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("themes");
    let output_dir = tempfile::tempdir().expect("failed to create temp benchmark directory");

    let mut source_files: Vec<PathBuf> = fs::read_dir(&source_dir)
        .expect("failed to read source themes")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.extension().and_then(|extension| extension.to_str()) == Some("json")
        })
        .collect();
    source_files.sort();

    let mut theme_payloads = Vec::new();
    for file in source_files {
        let stem = file
            .file_stem()
            .and_then(|value| value.to_str())
            .expect("invalid source theme file name")
            .to_string();
        let content = fs::read_to_string(&file).expect("failed to read source theme JSON");
        theme_payloads.push((stem, content));
    }

    // Produce enough files to make timing differences visible even on warm caches.
    let copies_per_theme = 80usize;
    for copy_index in 0..copies_per_theme {
        for (stem, content) in &theme_payloads {
            let file_name = format!("{stem}-{copy_index}.json");
            let destination = output_dir.path().join(file_name);
            fs::write(destination, content).expect("failed to write benchmark theme JSON");
        }
    }

    output_dir
}

fn run_benchmark_10_times(paths: &[PathBuf]) -> BenchmarkSummary {
    let mut sequential_runs = Vec::with_capacity(10);
    let mut parallel_runs = Vec::with_capacity(10);

    for _ in 0..10 {
        let seq_start = Instant::now();
        let seq_parsed = sequential_read_and_parse(paths);
        sequential_runs.push(seq_start.elapsed());

        let par_start = Instant::now();
        let par_parsed = parallel_read_then_parse(paths);
        parallel_runs.push(par_start.elapsed());

        assert_eq!(
            seq_parsed, par_parsed,
            "sequential and parallel flows parsed different file counts"
        );
    }

    BenchmarkSummary {
        sequential_runs,
        parallel_runs,
    }
}

fn total_ms(samples: &[Duration]) -> f64 {
    samples.iter().map(Duration::as_secs_f64).sum::<f64>() * 1000.0
}

fn avg_ms(samples: &[Duration]) -> f64 {
    total_ms(samples) / samples.len() as f64
}

#[test]
fn benchmark_theme_loading_sequential_vs_parallel_for_10_runs() {
    let benchmark_dir = setup_benchmark_themes();
    let paths = collect_json_paths(benchmark_dir.path());
    assert!(!paths.is_empty(), "benchmark input should not be empty");

    let summary = run_benchmark_10_times(&paths);
    let sequential_total = total_ms(&summary.sequential_runs);
    let parallel_total = total_ms(&summary.parallel_runs);
    let delta_ms = sequential_total - parallel_total;
    let speedup = if parallel_total > 0.0 {
        sequential_total / parallel_total
    } else {
        f64::INFINITY
    };

    eprintln!("Theme load benchmark input files: {}", paths.len());
    eprintln!(
        "Sequential total (10 runs): {:.2} ms | avg: {:.2} ms/run",
        sequential_total,
        avg_ms(&summary.sequential_runs)
    );
    eprintln!(
        "Parallel total (10 runs):   {:.2} ms | avg: {:.2} ms/run",
        parallel_total,
        avg_ms(&summary.parallel_runs)
    );
    eprintln!(
        "Delta (seq - parallel):     {:.2} ms | speedup: {:.2}x",
        delta_ms, speedup
    );
}
