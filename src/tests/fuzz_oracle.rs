
use crate::{Flags, Regex};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

const ALPHABET: &[char] = &['a', 'b', 'c', '0', '1', ' '];
const META: &str = "\\^$.|?*+()[]{}";

fn gen_literal(rng: &mut StdRng) -> String {
    let c = ALPHABET[rng.gen_range(0..ALPHABET.len())];
    if META.contains(c) {
        format!("\\{c}")
    } else {
        c.to_string()
    }
}

fn gen_class(rng: &mut StdRng) -> &'static str {
    match rng.gen_range(0..6) {
        0 => r"\d",
        1 => r"\D",
        2 => r"\w",
        3 => r"\W",
        4 => r"\s",
        _ => r"\S",
    }
}

fn gen_set(rng: &mut StdRng) -> String {
    let n = rng.gen_range(1..=3);
    let mut s = String::from("[");
    if rng.gen_bool(0.3) {
        s.push('^');
    }
    for _ in 0..n {
        if rng.gen_bool(0.3) {
            let lo = b'a' + rng.gen_range(0..3);
            let hi = lo + rng.gen_range(0..3);
            s.push(lo as char);
            s.push('-');
            s.push(hi as char);
        } else {
            s.push(ALPHABET[rng.gen_range(0..ALPHABET.len())]);
        }
    }
    s.push(']');
    s
}

fn gen_quantifier(rng: &mut StdRng) -> String {
    let q = match rng.gen_range(0..7) {
        0 => String::new(),
        1 => "*".to_string(),
        2 => "+".to_string(),
        3 => "?".to_string(),
        4 => format!("{{{}}}", rng.gen_range(0..=3)),
        5 => format!("{{{},}}", rng.gen_range(0..=2)),
        _ => {
            let m = rng.gen_range(0..=2);
            let n = m + rng.gen_range(0..=2);
            format!("{{{m},{n}}}")
        }
    };
    if !q.is_empty() && rng.gen_bool(0.3) {
        format!("{q}?")
    } else {
        q
    }
}

fn gen_atom(rng: &mut StdRng, depth: u32) -> String {
    if depth == 0 {
        return gen_literal(rng);
    }
    match rng.gen_range(0..5) {
        0 => gen_literal(rng),
        1 => gen_class(rng).to_string(),
        2 => gen_set(rng),
        3 => ".".to_string(),
        _ => {
            let body = gen_alternation(rng, depth - 1);
            if rng.gen_bool(0.5) {
                format!("({body})")
            } else {
                format!("(?:{body})")
            }
        }
    }
}

fn gen_sequence(rng: &mut StdRng, depth: u32) -> String {
    let n = rng.gen_range(1..=4);
    let mut s = String::new();
    for _ in 0..n {
        s.push_str(&gen_atom(rng, depth));
        s.push_str(&gen_quantifier(rng));
    }
    s
}

fn gen_alternation(rng: &mut StdRng, depth: u32) -> String {
    let n = rng.gen_range(1..=2);
    (0..n)
        .map(|_| gen_sequence(rng, depth))
        .collect::<Vec<_>>()
        .join("|")
}

/// `pub(crate)`: reused by `fuzz_oracle_linear.rs` to generate the same
/// grammar for cross-checking the linear engine against this one.
pub(crate) fn gen_pattern(rng: &mut StdRng) -> String {
    let mut p = gen_alternation(rng, 3);
    if rng.gen_bool(0.15) {
        p = format!("^{p}");
    }
    if rng.gen_bool(0.15) {
        p = format!("{p}$");
    }
    if rng.gen_bool(0.1) {
        p = format!(r"\b{p}");
    }
    p
}

pub(crate) fn gen_haystack(rng: &mut StdRng) -> String {
    let len = rng.gen_range(0..=25);
    (0..len)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())])
        .collect()
}

fn oracle_for(pattern: &str) -> Option<regex::Regex> {
    let case_insensitive = !pattern.chars().any(|c| c.is_uppercase());
    regex::RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .ok()
}

fn run_fuzz_iterations(seed: u64, iterations: u32, known_pre_existing_mismatches: usize) {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut checked = 0u32;

    let flags = Flags {
        max_backtrack_steps: Some(150_000),
        ..Flags::default()
    };
    let big_budget_flags = Flags {
        max_backtrack_steps: Some(50_000_000),
        ..Flags::default()
    };

    let mut mismatches: Vec<String> = Vec::new();

    for i in 0..iterations {
        let pattern = gen_pattern(&mut rng);
        let haystack = gen_haystack(&mut rng);

        let Ok(monster) = Regex::new(&pattern, flags) else {
            continue;
        };
        let Some(oracle) = oracle_for(&pattern) else {
            continue;
        };

        let monster_matches: Vec<(usize, usize)> = monster
            .find_all(&haystack)
            .map(|m| (m.start, m.end))
            .collect();
        let oracle_matches: Vec<(usize, usize)> = oracle
            .find_iter(&haystack)
            .map(|m| (m.start(), m.end()))
            .collect();

        if monster_matches != oracle_matches {
            let monster_big: Vec<(usize, usize)> = Regex::new(&pattern, big_budget_flags)
                .unwrap()
                .find_all(&haystack)
                .map(|m| (m.start, m.end))
                .collect();
            if monster_big != oracle_matches {
                mismatches.push(format!(
                    "seed={seed} iter={i} pattern={pattern:?} haystack={haystack:?}\n\
                     monster={monster_matches:?}\nregex_crate={oracle_matches:?}"
                ));
            }
        }
        checked += 1;
    }

    assert!(
        checked > iterations / 2,
        "too many generated patterns were skipped as invalid on one side or \
         the other (checked {checked}/{iterations}) - grammar may be miscalibrated"
    );

    assert!(
        mismatches.len() <= known_pre_existing_mismatches,
        "{} / {checked} generated cases disagreed with the regex crate oracle \
         (expected at most {known_pre_existing_mismatches} known pre-existing \
         ones) - first few:\n\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n\n")
    );
}

#[test]
fn fuzz_oracle_backtracking_vs_regex_crate() {
    run_fuzz_iterations(0xC0FFEE, 800, 0);
}

#[test]
#[ignore]
fn fuzz_oracle_backtracking_vs_regex_crate_heavy() {
    run_fuzz_iterations(0xC0FFEE, 150_000, 515);
}
