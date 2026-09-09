// Regexes are constructed once per input size (outside the timed `b.iter` loop),
// so the in-loop construction the lint flags is not on the hot path.
#![allow(clippy::regex_creation_in_loops)]

/// Comparison benchmarks: monster-regex linear engine vs the `regex` crate.
/// Run with: cargo bench --bench vs_regex
///
/// Document sizes: 1 KB, 10 KB, 100 KB, 1 MB, 10 MB
/// Patterns tested:
///   1. literal       – "fn"         (letter pair, high frequency)
///   2. digit_date    – \d{4}-\d{2}-\d{2}
///   3. word_class    – \w+@\w+\.\w+  (simple email-like)
///   4. dna_complex   – [AC]+G+[TA]+
///   5. ipv4          – \d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}
///   6. http_method   – GET|POST|PUT|DELETE  (alternation, differing branch lengths)
///   7. log_line      – \[\d{4}-\d{2}-\d{2}\] \w+: .*  (mixed shape, unbounded `.*` tail)
///   8. word_boundary – \bfoo\b
///   9. unicode_word  – \w+ over non-ASCII (accented Latin) text
///   10. sparse_literal – "needle" appearing once near the end of a large document
use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use monster_regex::{Flags, Regex};

// ── helpers ──────────────────────────────────────────────────────────────────

/// Builds a `regex` crate `Regex` with the same case-sensitivity
/// monster-regex's smartcase would pick for this pattern (case-insensitive
/// whenever the pattern has no uppercase letter at all - true of most of the
/// patterns below, since `\d`, `\w`, `foo`, etc. are all lowercase). `regex`
/// has no smartcase of its own and is case-sensitive by default, so without
/// this every benchmark using such a pattern would silently compare
/// monster-regex's case-insensitive match against `regex`'s case-sensitive
/// one - a real cost difference, not a fair reading of either engine's
/// per-byte speed.
fn regex_crate_matching_case(pattern: &str) -> regex::Regex {
    let case_insensitive = !pattern.chars().any(|c| c.is_uppercase());
    regex::RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .unwrap()
}

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

fn make_ipv4_text(target_bytes: usize) -> String {
    let chunk = "Request from 192.168.1.1 to 10.0.0.255 via 8.8.8.8 and 172.16.254.1\n";
    let reps = (target_bytes / chunk.len()).max(1);
    chunk.repeat(reps)
}

fn make_http_log_text(target_bytes: usize) -> String {
    let chunk = "GET /index.html 200\nPOST /api/users 201\nHEAD /favicon.ico 404\nPUT /api/users/5 200\nOPTIONS /api/users 204\nDELETE /api/users/5 204\nPATCH /api/users/5 200\n";
    let reps = (target_bytes / chunk.len()).max(1);
    chunk.repeat(reps)
}

fn make_log_line_text(target_bytes: usize) -> String {
    let chunk = "[2024-01-15] INFO: Server started successfully on port 8080\n[2024-01-15] ERROR: Connection refused by upstream host\n[2024-01-16] DEBUG: Cache miss for key user:1234 falling back to db\n[2024-01-16] WARN: Retrying request after timeout\n";
    let reps = (target_bytes / chunk.len()).max(1);
    chunk.repeat(reps)
}

fn make_word_boundary_text(target_bytes: usize) -> String {
    let chunk = "foo bar foobar barfoo foo baz foo123 xfoo yfoo foo end\n";
    let reps = (target_bytes / chunk.len()).max(1);
    chunk.repeat(reps)
}

fn make_unicode_text(target_bytes: usize) -> String {
    let chunk = "héllo wörld café mañana über résumé naïve façade cliché\n";
    // Repeat by char-count target rather than byte-count, since this chunk is
    // multibyte UTF-8 - close enough for a throughput benchmark.
    let reps = (target_bytes / chunk.len()).max(1);
    chunk.repeat(reps)
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
            let re = regex_crate_matching_case("fn");
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
            let re = regex_crate_matching_case(r"\d{4}-\d{2}-\d{2}");
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
            let re = regex_crate_matching_case(r"\w+@\w+\.\w+");
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
            let re = regex_crate_matching_case(r"[AC]+G+[TA]+");
            b.iter(|| re.find_iter(black_box(text)).count())
        });
    }
    group.finish();
}

// 5. IPv4 address \d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}
fn bench_ipv4(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipv4_pattern");
    group.sample_size(20);

    for &sz in SIZES {
        let input = make_ipv4_text(sz);
        group.throughput(Throughput::Bytes(input.len() as u64));
        let label = size_label(sz);

        group.bench_with_input(BenchmarkId::new("monster", label), &input, |b, text| {
            let re =
                Regex::new_linear(r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}", Flags::default()).unwrap();
            b.iter(|| re.find_all(black_box(text)).count())
        });

        group.bench_with_input(BenchmarkId::new("regex_crate", label), &input, |b, text| {
            let re = regex_crate_matching_case(r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}");
            b.iter(|| re.find_iter(black_box(text)).count())
        });
    }
    group.finish();
}

// 6. HTTP method alternation GET|POST|PUT|DELETE (differing branch lengths -
// not fixed-length, not a disjoint-segments concatenation, so this exercises
// the origin-tracking bit-parallel path rather than either fast path).
fn bench_http_method(c: &mut Criterion) {
    let mut group = c.benchmark_group("http_method_alt");
    group.sample_size(20);

    for &sz in SIZES {
        let input = make_http_log_text(sz);
        group.throughput(Throughput::Bytes(input.len() as u64));
        let label = size_label(sz);

        group.bench_with_input(BenchmarkId::new("monster", label), &input, |b, text| {
            let re = Regex::new_linear(r"GET|POST|PUT|DELETE", Flags::default()).unwrap();
            b.iter(|| re.find_all(black_box(text)).count())
        });

        group.bench_with_input(BenchmarkId::new("regex_crate", label), &input, |b, text| {
            let re = regex_crate_matching_case(r"GET|POST|PUT|DELETE");
            b.iter(|| re.find_iter(black_box(text)).count())
        });
    }
    group.finish();
}

// 7. Log line \[\d{4}-\d{2}-\d{2}\] \w+: .*  (mixed shape: fixed-length date
// bracket, then an unbounded `.*` tail whose near-universal alphabet makes it
// ineligible for the disjoint-segments fast path - realistic "doesn't get a
// fast path" case).
fn bench_log_line(c: &mut Criterion) {
    let mut group = c.benchmark_group("log_line");
    group.sample_size(20);

    for &sz in SIZES {
        let input = make_log_line_text(sz);
        group.throughput(Throughput::Bytes(input.len() as u64));
        let label = size_label(sz);

        group.bench_with_input(BenchmarkId::new("monster", label), &input, |b, text| {
            let re = Regex::new_linear(r"\[\d{4}-\d{2}-\d{2}\] \w+: .*", Flags::default()).unwrap();
            b.iter(|| re.find_all(black_box(text)).count())
        });

        group.bench_with_input(BenchmarkId::new("regex_crate", label), &input, |b, text| {
            let re = regex_crate_matching_case(r"\[\d{4}-\d{2}-\d{2}\] \w+: .*");
            b.iter(|| re.find_iter(black_box(text)).count())
        });
    }
    group.finish();
}

// 8. Word boundary \bfoo\b (WordBoundary states disqualify the bit-parallel
// path entirely - exercises the char-based baseline PikeVM instead).
fn bench_word_boundary(c: &mut Criterion) {
    let mut group = c.benchmark_group("word_boundary");
    group.sample_size(20);

    for &sz in SIZES {
        let input = make_word_boundary_text(sz);
        group.throughput(Throughput::Bytes(input.len() as u64));
        let label = size_label(sz);

        group.bench_with_input(BenchmarkId::new("monster", label), &input, |b, text| {
            let re = Regex::new_linear(r"\bfoo\b", Flags::default()).unwrap();
            b.iter(|| re.find_all(black_box(text)).count())
        });

        group.bench_with_input(BenchmarkId::new("regex_crate", label), &input, |b, text| {
            let re = regex_crate_matching_case(r"\bfoo\b");
            b.iter(|| re.find_iter(black_box(text)).count())
        });
    }
    group.finish();
}

// 9. Unicode \w+ over non-ASCII (accented Latin) text - the haystack is never
// all-ASCII, so the ASCII-gated bit-parallel fast path never engages on
// either side of this benchmark; shows the genuine cost of falling back to
// the general engine on real Unicode input.
fn bench_unicode_word(c: &mut Criterion) {
    let mut group = c.benchmark_group("unicode_word");
    group.sample_size(20);

    for &sz in SIZES {
        let input = make_unicode_text(sz);
        group.throughput(Throughput::Bytes(input.len() as u64));
        let label = size_label(sz);

        group.bench_with_input(BenchmarkId::new("monster", label), &input, |b, text| {
            let re = Regex::new_linear(r"\w+", Flags::default()).unwrap();
            b.iter(|| re.find_all(black_box(text)).count())
        });

        group.bench_with_input(BenchmarkId::new("regex_crate", label), &input, |b, text| {
            let re = regex_crate_matching_case(r"\w+");
            b.iter(|| re.find_iter(black_box(text)).count())
        });
    }
    group.finish();
}

// 10. Sparse literal search: a specific, rare term appearing once near the
// end of an otherwise non-matching document - "find where I used this
// identifier" rather than "find this common keyword everywhere," which
// `literal_fn` above already covers. Dense keyword search interrupts a
// SIMD scan on almost every chunk (little chance for wider registers to pay
// off); a rare term instead spends nearly the whole scan on chunks with no
// candidate at all, which is exactly where a wider SIMD kernel should win.
fn make_sparse_needle_text(target_bytes: usize) -> String {
    let filler = "the quick brown fox jumps over the lazy dog and runs away ";
    let reps = (target_bytes / filler.len()).max(1);
    let mut s = filler.repeat(reps);
    s.push_str("needle");
    s
}

fn bench_sparse_literal(c: &mut Criterion) {
    let mut group = c.benchmark_group("sparse_literal");
    group.sample_size(20);

    for &sz in SIZES {
        let input = make_sparse_needle_text(sz);
        group.throughput(Throughput::Bytes(input.len() as u64));
        let label = size_label(sz);

        group.bench_with_input(BenchmarkId::new("monster", label), &input, |b, text| {
            let re = Regex::new_linear("needle", Flags::default()).unwrap();
            b.iter(|| re.find_all(black_box(text)).count())
        });

        group.bench_with_input(BenchmarkId::new("regex_crate", label), &input, |b, text| {
            let re = regex_crate_matching_case("needle");
            b.iter(|| re.find_iter(black_box(text)).count())
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_literal,
    bench_date,
    bench_email,
    bench_dna,
    bench_ipv4,
    bench_http_method,
    bench_log_line,
    bench_word_boundary,
    bench_unicode_word,
    bench_sparse_literal
);
criterion_main!(benches);
