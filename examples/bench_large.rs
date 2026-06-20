use std::alloc::{GlobalAlloc, Layout, System};
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Instant;

use hdrhistogram::Histogram;

use sled::{Config, Db};

// --- Peak-memory tracking allocator ---

// Total bytes ever allocated — reported in baseline output alongside MAX_RESIDENT
static ALLOCATED: AtomicU64 = AtomicU64::new(0);
static RESIDENT: AtomicU64 = AtomicU64::new(0);
static MAX_RESIDENT: AtomicU64 = AtomicU64::new(0);

struct BenchAllocator;

unsafe impl GlobalAlloc for BenchAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let size = layout.size() as u64;
            ALLOCATED.fetch_add(size, Ordering::Relaxed);
            let new_resident =
                RESIDENT.fetch_add(size, Ordering::Relaxed) + size;
            MAX_RESIDENT.fetch_max(new_resident, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        RESIDENT.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: BenchAllocator = BenchAllocator;

// --- Database helpers ---

const DB_PATH: &str = "profile.bench_large.sled";
const LEAF_FANOUT: usize = 3;
const N_KEYS: u64 = 10_000_000;
const N_THREADS: usize = 2;
const KEYS_PER_THREAD: u64 = N_KEYS / N_THREADS as u64;

type BenchDb = Db<LEAF_FANOUT>;

fn open_db() -> BenchDb {
    Config {
        path: DB_PATH.into(),
        cache_capacity_bytes: 512 * 1024 * 1024,
        flush_every_ms: Some(200),
        ..Config::default()
    }
    .open()
    .unwrap()
}

fn new_histogram() -> Histogram<u64> {
    // Tracks insert latency in microseconds (1us to 60s, 3 significant figures).
    // Latencies above 60,000,000us (60s) saturate silently at the max bucket.
    Histogram::<u64>::new_with_bounds(1, 60_000_000, 3).unwrap()
}

fn git_hash() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn os_info() -> String {
    std::process::Command::new("uname")
        .arg("-a")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn cpu_info() -> String {
    // macOS
    let mac = std::process::Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(info) = mac {
        return info;
    }

    // Linux
    std::fs::read_to_string("/proc/cpuinfo")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("model name"))
        .map(|l| l.splitn(2, ':').nth(1).unwrap_or("").trim().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn write_baseline(
    write_duration: std::time::Duration,
    throughput: f64,
    hist: &Histogram<u64>,
    flush_duration: std::time::Duration,
    recovery_duration: std::time::Duration,
    max_resident_bytes: u64,
    total_allocated_bytes: u64,
) {
    use std::fmt::Write as FmtWrite;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut out = String::new();
    writeln!(out, "=== sled bench_large baseline ===").unwrap();
    writeln!(out, "Date (unix): {}", now).unwrap();
    writeln!(out, "Git: {}", git_hash()).unwrap();
    writeln!(out, "OS: {}", os_info()).unwrap();
    writeln!(out, "CPU: {}", cpu_info()).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "Parameters:").unwrap();
    writeln!(out, "  LEAF_FANOUT:  {}", LEAF_FANOUT).unwrap();
    writeln!(out, "  N_KEYS:       {}", N_KEYS).unwrap();
    writeln!(out, "  N_THREADS:    {}", N_THREADS).unwrap();
    writeln!(out, "  Note: LEAF_FANOUT={} is a stress-test configuration (minimum allowed).", LEAF_FANOUT).unwrap();
    writeln!(out, "        Default is 1024. Lower values increase index overhead and memory pressure.").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "Write throughput:").unwrap();
    writeln!(out, "  Duration:     {:.2?}", write_duration).unwrap();
    writeln!(out, "  Throughput:   {:.0} keys/sec", throughput).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "Per-insert latency (microseconds):").unwrap();
    writeln!(out, "  p50:          {}", hist.value_at_quantile(0.50)).unwrap();
    writeln!(out, "  p95:          {}", hist.value_at_quantile(0.95)).unwrap();
    writeln!(out, "  p99:          {}", hist.value_at_quantile(0.99)).unwrap();
    writeln!(out, "  p99.9:        {}", hist.value_at_quantile(0.999)).unwrap();
    writeln!(out, "  max:          {}", hist.max()).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "Flush:").unwrap();
    writeln!(out, "  Duration:     {:.2?}", flush_duration).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "Recovery:").unwrap();
    writeln!(out, "  Duration:     {:.2?}", recovery_duration).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "Memory (peak during write phase):").unwrap();
    writeln!(
        out,
        "  Peak resident: {} bytes ({:.1} MiB)",
        max_resident_bytes,
        max_resident_bytes as f64 / (1024.0 * 1024.0),
    )
    .unwrap();
    writeln!(
        out,
        "  Total allocated: {} bytes ({:.1} MiB)",
        total_allocated_bytes,
        total_allocated_bytes as f64 / (1024.0 * 1024.0),
    )
    .unwrap();

    std::fs::create_dir_all("benchmarks").unwrap();
    std::fs::write("benchmarks/baseline.txt", &out).unwrap();
    print!("{}", out);
    println!("Baseline written to benchmarks/baseline.txt");
}

fn main() {
    let _ = fs::remove_dir_all(DB_PATH);
    let db = open_db();

    let barrier = Arc::new(Barrier::new(N_THREADS + 1));
    let shared_hist = Arc::new(Mutex::new(new_histogram()));

    // Initialized with a placeholder Instant; overwritten inside thread::scope before use.
    // Uninit let bindings don't work here because the compiler can't prove initialization
    // through closure captures.
    let mut write_start = Instant::now();
    let mut write_end = Instant::now();

    thread::scope(|s| {
        let handles: Vec<_> = (0..N_THREADS)
            .map(|thread_id| {
                let db = db.clone();
                let barrier = Arc::clone(&barrier);
                let shared_hist = Arc::clone(&shared_hist);

                s.spawn(move || {
                    let start_key = thread_id as u64 * KEYS_PER_THREAD;
                    let end_key = start_key + KEYS_PER_THREAD;
                    let mut local_hist = new_histogram();

                    barrier.wait();

                    for key in start_key..end_key {
                        let key_bytes = key.to_le_bytes();
                        let value_bytes = [0u8; 8];

                        let t0 = Instant::now();
                        db.insert(key_bytes, value_bytes).unwrap();
                        let elapsed_us = t0.elapsed().as_micros() as u64;
                        local_hist.record(elapsed_us.max(1)).unwrap();
                    }

                    shared_hist.lock().unwrap().add(&local_hist).unwrap();
                })
            })
            .collect();

        write_start = Instant::now();
        barrier.wait(); // release all writer threads simultaneously

        for h in handles {
            h.join().expect("worker thread panicked");
        }
        write_end = Instant::now();
    });

    let write_duration = write_end.duration_since(write_start);
    let throughput = N_KEYS as f64 / write_duration.as_secs_f64();

    // Snapshot peak memory before flush/recovery phases add to it
    let max_resident = MAX_RESIDENT.load(Ordering::Relaxed);
    let total_allocated = ALLOCATED.load(Ordering::Relaxed);

    // Flush phase
    let flush_start = Instant::now();
    db.flush().unwrap();
    let flush_duration = flush_start.elapsed();

    // Recovery phase — drop the db, reopen from same path
    // Use flush_every_ms: None to isolate recovery measurement from background task startup
    drop(db);
    let recovery_start = Instant::now();
    let _recovered_db = Config {
        path: DB_PATH.into(),
        cache_capacity_bytes: 512 * 1024 * 1024,
        flush_every_ms: None,
        ..Config::default()
    }
    .open::<LEAF_FANOUT>()
    .unwrap();
    let recovery_duration = recovery_start.elapsed();

    let hist = shared_hist.lock().unwrap();
    write_baseline(
        write_duration,
        throughput,
        &hist,
        flush_duration,
        recovery_duration,
        max_resident,
        total_allocated,
    );
}
