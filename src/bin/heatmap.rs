use monster_regex::{Flags, Regex};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

// Custom allocator to track memory usage
struct TrackingAllocator;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATED: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            let size = layout.size();
            let old_alloc = ALLOCATED.fetch_add(size, Ordering::Relaxed);
            let current = old_alloc + size - DEALLOCATED.load(Ordering::Relaxed);

            // Update peak
            let mut old_peak = PEAK.load(Ordering::Relaxed);
            while current > old_peak {
                match PEAK.compare_exchange_weak(
                    old_peak,
                    current,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(x) => old_peak = x,
                }
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: TrackingAllocator = TrackingAllocator;

fn reset_tracking() {
    ALLOCATED.store(0, Ordering::Relaxed);
    DEALLOCATED.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
}

fn get_stats() -> (usize, usize, usize) {
    let allocated = ALLOCATED.load(Ordering::Relaxed);
    let deallocated = DEALLOCATED.load(Ordering::Relaxed);
    let peak = PEAK.load(Ordering::Relaxed);
    (allocated, deallocated, peak)
}

#[derive(Debug)]
struct ProfileResult {
    name: String,
    time_ns: u64,
    total_allocated: usize,
    peak_memory: usize,
    iterations: usize,
}

fn profile_operation<F>(name: &str, iterations: usize, mut f: F) -> ProfileResult
where
    F: FnMut(),
{
    const RUNS: usize = 50; // Run 50 times for averaging

    let mut times = Vec::with_capacity(RUNS);
    let mut allocs = Vec::with_capacity(RUNS);
    let mut peaks = Vec::with_capacity(RUNS);

    for _ in 0..RUNS {
        // Warmup
        for _ in 0..3 {
            f();
        }

        reset_tracking();
        let start = Instant::now();

        for _ in 0..iterations {
            f();
        }

        let elapsed = start.elapsed();
        let (allocated, _, peak) = get_stats();

        times.push(elapsed.as_nanos() as u64 / iterations as u64);
        allocs.push(allocated / iterations);
        peaks.push(peak);
    }

    // Sort and take median to avoid outliers
    times.sort();
    allocs.sort();
    peaks.sort();

    ProfileResult {
        name: name.to_string(),
        time_ns: times[RUNS / 2], // Median
        total_allocated: allocs[RUNS / 2],
        peak_memory: peaks[RUNS / 2],
        iterations,
    }
}

fn generate_large_rust_code(lines: usize) -> String {
    let mut code = String::with_capacity(lines * 50);
    for i in 0..lines {
        code.push_str(&format!("fn function_{}() -> Result<(), Error> {{\n", i));
        code.push_str("    let x = 42;\n");
        code.push_str("    if x > 10 {\n");
        code.push_str("        return Ok(());\n");
        code.push_str("    }\n");
        code.push_str("    return Err(Error::new());\n");
        code.push_str("}\n\n");
    }
    code
}

fn generate_dna(len: usize) -> String {
    let mut dna = String::with_capacity(len);
    let chars = ['A', 'C', 'G', 'T'];
    for i in 0..len {
        dna.push(chars[i % 4]);
    }
    dna
}

fn render_bar(value: usize, max_value: usize, width: usize) -> String {
    let filled = if max_value > 0 {
        (value as f64 / max_value as f64 * width as f64) as usize
    } else {
        0
    };

    let bar_chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let full_blocks = filled / 8;
    let remainder = filled % 8;

    let mut bar = String::new();
    for _ in 0..full_blocks {
        bar.push('█');
    }
    if remainder > 0 && bar.len() < width {
        bar.push(bar_chars[remainder - 1]);
    }
    while bar.len() < width {
        bar.push(' ');
    }

    bar
}

fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_time(ns: u64) -> String {
    if ns < 1_000 {
        format!("{} ns", ns)
    } else if ns < 1_000_000 {
        format!("{:.1} µs", ns as f64 / 1_000.0)
    } else if ns < 1_000_000_000 {
        format!("{:.1} ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.2} s", ns as f64 / 1_000_000_000.0)
    }
}

fn main() {
    println!("Monster-Regex Performance Heat Map\n");

    let input_code = generate_large_rust_code(1000);
    let input_dna = generate_dna(10_000);

    let mut results = Vec::new();

    // Profile different patterns and engines
    println!("Profiling Linear Engine...");

    let re = Regex::new_linear(r"fn.*\n.*return", Flags::default()).unwrap();
    results.push(profile_operation("Linear: fn multiline", 50, || {
        let _ = re.find_all(&input_code).count();
    }));

    let re = Regex::new_linear("fn", Flags::default()).unwrap();
    results.push(profile_operation("Linear: literal 'fn'", 100, || {
        let _ = re.find_all(&input_code).count();
    }));

    let re = Regex::new_linear(r"\d{4}-\d{2}-\d{2}", Flags::default()).unwrap();
    let date_input = "Date: 2024-01-01, IP: 192.168.0.1\n".repeat(1000);
    results.push(profile_operation("Linear: date pattern", 100, || {
        let _ = re.find_all(&date_input).count();
    }));

    let re = Regex::new_linear(r"[AC]+G+[TA]+", Flags::default()).unwrap();
    results.push(profile_operation("Linear: DNA complex", 100, || {
        let _ = re.find_all(&input_dna).count();
    }));

    println!("Profiling Backtracking Engine...");

    let re = Regex::new(r"fn.*\n.*return", Flags::default()).unwrap();
    results.push(profile_operation("Backtrack: fn multiline", 50, || {
        let _ = re.find_all(&input_code).count();
    }));

    let re = Regex::new("fn", Flags::default()).unwrap();
    results.push(profile_operation("Backtrack: literal 'fn'", 100, || {
        let _ = re.find_all(&input_code).count();
    }));

    let re = Regex::new(r"\d{4}-\d{2}-\d{2}", Flags::default()).unwrap();
    results.push(profile_operation("Backtrack: date pattern", 100, || {
        let _ = re.find_all(&date_input).count();
    }));

    let re = Regex::new(r"[AC]+G+[TA]+", Flags::default()).unwrap();
    results.push(profile_operation("Backtrack: DNA complex", 100, || {
        let _ = re.find_all(&input_dna).count();
    }));

    // Calculate max values for normalization
    let max_time = results.iter().map(|r| r.time_ns).max().unwrap_or(1);
    let max_mem = results.iter().map(|r| r.total_allocated).max().unwrap_or(1);
    let max_peak = results.iter().map(|r| r.peak_memory).max().unwrap_or(1);

    println!("\n{}", "=".repeat(100));
    println!("PERFORMANCE HEAT MAP");
    println!("{}", "=".repeat(100));

    println!("\nTIME PER OPERATION");
    println!("{:-<100}", "");
    for result in &results {
        let bar = render_bar(result.time_ns as usize, max_time as usize, 40);
        println!(
            "{:<30} │{}│ {}",
            result.name,
            bar,
            format_time(result.time_ns)
        );
    }

    println!("\nMEMORY ALLOCATED PER OPERATION");
    println!("{:-<100}", "");
    for result in &results {
        let bar = render_bar(result.total_allocated, max_mem, 40);
        println!(
            "{:<30} │{}│ {}",
            result.name,
            bar,
            format_bytes(result.total_allocated)
        );
    }

    println!("\nPEAK MEMORY USAGE");
    println!("{:-<100}", "");
    for result in &results {
        let bar = render_bar(result.peak_memory, max_peak, 40);
        println!(
            "{:<30} │{}│ {}",
            result.name,
            bar,
            format_bytes(result.peak_memory)
        );
    }

    // Comparison table
    println!("\nDETAILED COMPARISON");
    println!("{:-<100}", "");
    println!(
        "{:<30} {:>15} {:>20} {:>20}",
        "Operation", "Time", "Alloc/Op", "Peak Memory"
    );
    println!("{:-<100}", "");

    for result in &results {
        println!(
            "{:<30} {:>15} {:>20} {:>20}",
            result.name,
            format_time(result.time_ns),
            format_bytes(result.total_allocated),
            format_bytes(result.peak_memory)
        );
    }

    // Performance ratios
    println!("\nLINEAR vs BACKTRACKING RATIOS");
    println!("{:-<100}", "");

    for i in 0..4 {
        let linear = &results[i];
        let backtrack = &results[i + 4];

        let time_ratio = linear.time_ns as f64 / backtrack.time_ns as f64;
        let mem_ratio = linear.total_allocated as f64 / backtrack.total_allocated.max(1) as f64;

        println!(
            "{:<25} Time: {:>6.2}x   Memory: {:>6.2}x",
            linear.name.replace("Linear: ", ""),
            time_ratio,
            mem_ratio
        );
    }

    println!("\n{}", "=".repeat(100));
}
