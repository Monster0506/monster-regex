use monster_regex::{Flags, Regex};
use std::env;
use std::time::Instant;

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

fn run_bench(name: &str, mut f: impl FnMut()) {
    // Warmup
    let start = Instant::now();
    let mut iterations = 0;
    while start.elapsed().as_millis() < 100 {
        f();
        iterations += 1;
    }

    // Measure
    let start = Instant::now();
    let mut count = 0;
    let duration = std::time::Duration::from_secs(2);

    while start.elapsed() < duration {
        f();
        count += 1;
    }
    let elapsed = start.elapsed();

    let iter_per_sec = count as f64 / elapsed.as_secs_f64();
    println!(
        "{:<30} : {:.2} iter/s ({} iters in {:.2?})",
        name, iter_per_sec, count, elapsed
    );
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let filter = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let is_linear = filter.contains("linear") || filter.is_empty();
    let is_backtrack = filter.contains("backtrack") || filter.is_empty();

    let input_code = generate_large_rust_code(1000);
    let input_dna = generate_dna(10_000);
    let pattern_stress = r"fn.*\n.*return";
    let pattern_dna = r"[AC]+G+[TA]+";

    println!("Starting benchmarks...");

    if is_linear {
        if filter.contains("stress") || filter.is_empty() {
            let re = Regex::new_linear(pattern_stress, Flags::default()).unwrap();
            run_bench("Linear: fn decl multiline", || {
                let _ = re.find_all(&input_code).count();
            });
        }

        if filter.contains("dna") || filter.is_empty() {
            let re = Regex::new_linear(pattern_dna, Flags::default()).unwrap();
            run_bench("Linear: DNA Complex", || {
                let _ = re.find_all(&input_dna).count();
            });
        }
    }

    if is_backtrack {
        if filter.contains("stress") || filter.is_empty() {
            let re = Regex::new(pattern_stress, Flags::default()).unwrap();
            run_bench("Backtrack: fn decl multiline", || {
                let _ = re.find_all(&input_code).count();
            });
        }

        if filter.contains("dna") || filter.is_empty() {
            let re = Regex::new(pattern_dna, Flags::default()).unwrap();
            run_bench("Backtrack: DNA Complex", || {
                let _ = re.find_all(&input_dna).count();
            });
        }
    }
}
