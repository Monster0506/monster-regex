
use super::fuzz_oracle::{gen_haystack, gen_pattern};
use crate::{Flags, Regex};
use rand::SeedableRng;
use rand::rngs::StdRng;

fn run_fuzz_iterations(seed: u64, iterations: u32, known_pre_existing_mismatches: usize) {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut checked = 0u32;

    let flags = Flags {
        max_backtrack_steps: Some(150_000),
        ..Flags::default()
    };
    // See `fuzz_oracle.rs`: only used to re-check an apparent mismatch
    // before counting it as one, so no impact on the common case's speed.
    let big_budget_flags = Flags {
        max_backtrack_steps: Some(50_000_000),
        ..Flags::default()
    };

    let mut mismatches: Vec<String> = Vec::new();

    for i in 0..iterations {
        let pattern = gen_pattern(&mut rng);
        let haystack = gen_haystack(&mut rng);

        let Ok(backtracking) = Regex::new(&pattern, flags) else {
            continue;
        };
        let Ok(linear) = Regex::new_linear(&pattern, flags) else {
            continue;
        };

        let backtracking_matches: Vec<(usize, usize)> = backtracking
            .find_all(&haystack)
            .map(|m| (m.start, m.end))
            .collect();
        let linear_matches: Vec<(usize, usize)> = linear
            .find_all(&haystack)
            .map(|m| (m.start, m.end))
            .collect();

        if backtracking_matches != linear_matches {
            let backtracking_big: Vec<(usize, usize)> = Regex::new(&pattern, big_budget_flags)
                .unwrap()
                .find_all(&haystack)
                .map(|m| (m.start, m.end))
                .collect();
            if backtracking_big != linear_matches {
                mismatches.push(format!(
                    "seed={seed} iter={i} pattern={pattern:?} haystack={haystack:?}\n\
                     backtracking={backtracking_matches:?}\nlinear={linear_matches:?}"
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
        "{} / {checked} generated cases disagreed between the backtracking and \
         linear engines (expected at most {known_pre_existing_mismatches} known \
         pre-existing ones) - first few:\n\n{}",
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
fn fuzz_oracle_linear_vs_backtracking() {
    run_fuzz_iterations(0xC0FFEE, 800, 6);
}

#[test]
#[ignore]
fn fuzz_oracle_linear_vs_backtracking_heavy() {
    run_fuzz_iterations(0xC0FFEE, 150_000, 1165);
}
