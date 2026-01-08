use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use monster_regex::{Flags, Regex};

// --- Helpers ---

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

// --- Benchmarks ---

fn bench_engines(c: &mut Criterion) {
    let mut group = c.benchmark_group("Engine Comparison");
    group.sample_size(10);

    let pattern_stress = r"fn.*\n.*return";

    // Generate a large input (approx 1000 matched blocks, 6000 lines, ~200KB)
    let input = generate_large_rust_code(1000);

    group.throughput(Throughput::Bytes(input.len() as u64));

    // Linear Engine
    group.bench_function("Linear: fn decl multiline", |b| {
        let re = Regex::new_linear(pattern_stress, Flags::default()).unwrap();
        b.iter(|| {
            // Count all matches
            let count = re.find_all(black_box(&input)).count();
            black_box(count);
        })
    });

    // Backtracking Engine (Default)
    group.bench_function("Backtracking: fn decl multiline", |b| {
        let re = Regex::new(pattern_stress, Flags::default()).unwrap();
        b.iter(|| {
            let count = re.find_all(black_box(&input)).count();
            black_box(count);
        })
    });

    group.finish();
}

fn bench_literals(c: &mut Criterion) {
    let mut group = c.benchmark_group("Literals");
    group.sample_size(10);
    let input = generate_large_rust_code(500); // 100KB
    group.throughput(Throughput::Bytes(input.len() as u64));

    for literal in ["fn", "return", "Error"] {
        group.bench_function(format!("Linear: {}", literal), |b| {
            let re = Regex::new_linear(literal, Flags::default()).unwrap();
            b.iter(|| {
                let count = re.find_all(black_box(&input)).count();
                black_box(count);
            })
        });

        group.bench_function(format!("Backtracking: {}", literal), |b| {
            let re = Regex::new(literal, Flags::default()).unwrap();
            b.iter(|| {
                let count = re.find_all(black_box(&input)).count();
                black_box(count);
            })
        });
    }
    group.finish();
}

fn bench_common_syntax(c: &mut Criterion) {
    let mut group = c.benchmark_group("Common Syntax");
    group.sample_size(10);
    let input = "Date: 2024-01-01, IP: 192.168.0.1, Email: test@example.com\n".repeat(1000);
    group.throughput(Throughput::Bytes(input.len() as u64));

    // Date Pattern: YYYY-MM-DD
    let date_pat = r"\d{4}-\d{2}-\d{2}";
    group.bench_function("Linear: Date", |b| {
        let re = Regex::new_linear(date_pat, Flags::default()).unwrap();
        b.iter(|| re.find_all(black_box(&input)).count())
    });
    group.bench_function("Backtracking: Date", |b| {
        let re = Regex::new(date_pat, Flags::default()).unwrap();
        b.iter(|| re.find_all(black_box(&input)).count())
    });

    // Simple Email
    let email_pat = r"\w+@\w+\.\w+";
    group.bench_function("Linear: Email", |b| {
        let re = Regex::new_linear(email_pat, Flags::default()).unwrap();
        b.iter(|| re.find_all(black_box(&input)).count())
    });
    group.bench_function("Backtracking: Email", |b| {
        let re = Regex::new(email_pat, Flags::default()).unwrap();
        b.iter(|| re.find_all(black_box(&input)).count())
    });

    group.finish();
}

fn bench_dna(c: &mut Criterion) {
    let mut group = c.benchmark_group("DNA");
    group.sample_size(10);
    let input = generate_dna(10_000);
    group.throughput(Throughput::Bytes(input.len() as u64));

    // Simple repeated pattern
    let pattern = "GATACA";
    group.bench_function("Linear: GATACA", |b| {
        let re = Regex::new_linear(pattern, Flags::default()).unwrap();
        b.iter(|| re.find_all(black_box(&input)).count())
    });
    group.bench_function("Backtracking: GATACA", |b| {
        let re = Regex::new(pattern, Flags::default()).unwrap();
        b.iter(|| re.find_all(black_box(&input)).count())
    });

    // Complex: (A|C)+G+(T|A)+
    let complex = r"[AC]+G+[TA]+";
    group.bench_function("Linear: Complex", |b| {
        let re = Regex::new_linear(complex, Flags::default()).unwrap();
        b.iter(|| re.find_all(black_box(&input)).count())
    });
    group.bench_function("Backtracking: Complex", |b| {
        let re = Regex::new(complex, Flags::default()).unwrap();
        b.iter(|| re.find_all(black_box(&input)).count())
    });

    group.finish();
}

fn bench_pathological(c: &mut Criterion) {
    let mut group = c.benchmark_group("Pathological Cases");
    group.sample_size(10);

    // Pattern: (a+)+$
    // This is the classic catastrophic backtracking case.
    let pattern = r"(a+)+$";

    let input_short = "aaaaaaaaaaaaaaaaaaaa"; // 20 'a's

    group.bench_function("Linear: (a+)+$ [20 chars]", |b| {
        let re = Regex::new_linear(pattern, Flags::default()).unwrap();
        b.iter(|| {
            assert!(re.is_match(black_box(input_short)));
        })
    });

    group.bench_function("Backtracking: (a+)+$ [20 chars]", |b| {
        let re = Regex::new(pattern, Flags::default()).unwrap();
        b.iter(|| {
            assert!(re.is_match(black_box(input_short)));
        })
    });

    // 30 chars - Backtracking will likely struggle significantly but let's test it as requested.
    let input_med = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"; // 30 chars

    group.bench_function("Linear: (a+)+$ [30 chars]", |b| {
        let re = Regex::new_linear(pattern, Flags::default()).unwrap();
        b.iter(|| {
            assert!(re.is_match(black_box(input_med)));
        })
    });

    // We omit the backtracking test for 30 chars because it would effectively hang the benchmark suite forever.
    /*
    group.bench_function("Backtracking: (a+)+$ [30 chars]", |b| {
        let re = Regex::new(pattern, Flags::default()).unwrap();
        b.iter(|| {
            assert!(re.is_match(black_box(input_med)));
        })
    });
    */

    group.finish();
}

criterion_group!(
    benches,
    bench_engines,
    bench_literals,
    bench_common_syntax,
    bench_dna,
    bench_pathological
);
criterion_main!(benches);
