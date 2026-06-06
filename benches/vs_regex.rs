// Regexes are constructed once per input size (outside the timed `b.iter` loop),
// so the in-loop construction the lint flags is not on the hot path.
#![allow(clippy::regex_creation_in_loops)]

/// Comparison benchmarks: monster-regex linear engine vs the `regex` crate.
/// Run with: cargo bench --bench vs_regex
///
/// Document sizes: 1 KB, 10 KB, 100 KB, 1 MB, 10 MB
/// Patterns tested:
///   1. literal     – "fn"         (letter pair, high frequency)
///   2. digit_date  – \d{4}-\d{2}-\d{2}
///   3. word_class  – \w+@\w+\.\w+  (simple email-like)
///   4. dna_complex – [AC]+G+[TA]+
use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use monster_regex::{Flags, Regex};

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_rust_code(target_bytes: usize) -> String {
    let chunk = "fn function_x() -> Result<(), Error> {\n    let x = 42;\n    if x > 10 {\n        return Ok(());\n    }\n    return Err(Error::new());\n}\n\n";
    let reps = (target_bytes / chunk.len()).max(1);
    chunk.repeat(reps)
}

fn make_date_text(target_bytes: usize) -> String {
    let chunk = "Date: 2024-01-01, next: 1999-12-31, also 2000-06-15\n";
    let reps = (target_bytes / chunk.len()).max(1);
    chunk.repeat(reps)
}

fn make_email_text(target_bytes: usize) -> String {
    let chunk = "contact user@example.com or admin@corp.org for info\n";
    let reps = (target_bytes / chunk.len()).max(1);
    chunk.repeat(reps)
}

fn make_dna(target_bytes: usize) -> String {
    let bases = b"ACGT";
    let mut s = String::with_capacity(target_bytes);
    for i in 0..target_bytes {
        s.push(bases[i % 4] as char);
    }
    s
}

// ── benchmark groups ─────────────────────────────────────────────────────────

const SIZES: &[usize] = &[
    1_024,      //  1 KB
    10_240,     // 10 KB
    102_400,    // 100 KB
    1_048_576,  //  1 MB
    10_485_760, // 10 MB
];

fn size_label(bytes: usize) -> &'static str {
    match bytes {
        1_024 => "1KB",
        10_240 => "10KB",
        102_400 => "100KB",
        1_048_576 => "1MB",
        10_485_760 => "10MB",
        _ => "?",
    }
}

// 1. Literal "fn"
fn bench_literal(c: &mut Criterion) {
    let mut group = c.benchmark_group("literal_fn");
    group.sample_size(20);

    for &sz in SIZES {
        let input = make_rust_code(sz);
        group.throughput(Throughput::Bytes(input.len() as u64));
        let label = size_label(sz);

        group.bench_with_input(BenchmarkId::new("monster", label), &input, |b, text| {
            let re = Regex::new_linear("fn", Flags::default()).unwrap();
            b.iter(|| re.find_all(black_box(text)).count())
        });

        group.bench_with_input(BenchmarkId::new("regex_crate", label), &input, |b, text| {
            let re = regex::Regex::new("fn").unwrap();
            b.iter(|| re.find_iter(black_box(text)).count())
        });
    }
    group.finish();
}

// 2. Date pattern \d{4}-\d{2}-\d{2}
fn bench_date(c: &mut Criterion) {
    let mut group = c.benchmark_group("date_pattern");
    group.sample_size(20);

    for &sz in SIZES {
        let input = make_date_text(sz);
        group.throughput(Throughput::Bytes(input.len() as u64));
        let label = size_label(sz);

        group.bench_with_input(BenchmarkId::new("monster", label), &input, |b, text| {
            let re = Regex::new_linear(r"\d{4}-\d{2}-\d{2}", Flags::default()).unwrap();
            b.iter(|| re.find_all(black_box(text)).count())
        });

        group.bench_with_input(BenchmarkId::new("regex_crate", label), &input, |b, text| {
            let re = regex::Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
            b.iter(|| re.find_iter(black_box(text)).count())
        });
    }
    group.finish();
}

// 3. Email-like \w+@\w+\.\w+
fn bench_email(c: &mut Criterion) {
    let mut group = c.benchmark_group("email_pattern");
    group.sample_size(20);

    for &sz in SIZES {
        let input = make_email_text(sz);
        group.throughput(Throughput::Bytes(input.len() as u64));
        let label = size_label(sz);

        group.bench_with_input(BenchmarkId::new("monster", label), &input, |b, text| {
            let re = Regex::new_linear(r"\w+@\w+\.\w+", Flags::default()).unwrap();
            b.iter(|| re.find_all(black_box(text)).count())
        });

        group.bench_with_input(BenchmarkId::new("regex_crate", label), &input, |b, text| {
            let re = regex::Regex::new(r"\w+@\w+\.\w+").unwrap();
            b.iter(|| re.find_iter(black_box(text)).count())
        });
    }
    group.finish();
}

// 4. DNA complex [AC]+G+[TA]+
fn bench_dna(c: &mut Criterion) {
    let mut group = c.benchmark_group("dna_complex");
    group.sample_size(20);

    for &sz in SIZES {
        let input = make_dna(sz);
        group.throughput(Throughput::Bytes(input.len() as u64));
        let label = size_label(sz);

        group.bench_with_input(BenchmarkId::new("monster", label), &input, |b, text| {
            let re = Regex::new_linear(r"[AC]+G+[TA]+", Flags::default()).unwrap();
            b.iter(|| re.find_all(black_box(text)).count())
        });

        group.bench_with_input(BenchmarkId::new("regex_crate", label), &input, |b, text| {
            let re = regex::Regex::new(r"[AC]+G+[TA]+").unwrap();
            b.iter(|| re.find_iter(black_box(text)).count())
        });
    }
    group.finish();
}

criterion_group!(benches, bench_literal, bench_date, bench_email, bench_dna);
criterion_main!(benches);
