
use super::fuzz_oracle::{gen_haystack, gen_pattern};
use super::streaming::ChunkedHaystack;
use crate::{Flags, Regex};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

fn random_chunks<'a>(rng: &mut StdRng, text: &'a str) -> Vec<&'a str> {
    let n = text.len();
    if n == 0 {
        return vec![text];
    }
    let num_cuts = rng.gen_range(0..=4).min(n.saturating_sub(1));
    let mut cuts: Vec<usize> = (0..num_cuts).map(|_| rng.gen_range(1..n)).collect();
    cuts.sort_unstable();
    cuts.dedup();

    let mut chunks = Vec::with_capacity(cuts.len() + 1);
    let mut start = 0;
    for &cut in &cuts {
        chunks.push(&text[start..cut]);
        start = cut;
    }
    chunks.push(&text[start..]);
    chunks
}

fn run_fuzz_iterations(seed: u64, iterations: u32) {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut checked = 0u32;

    let flags = Flags {
        max_backtrack_steps: Some(150_000),
        ..Flags::default()
    };

    let mut mismatches: Vec<String> = Vec::new();

    for i in 0..iterations {
        let pattern = gen_pattern(&mut rng);
        let haystack = gen_haystack(&mut rng);
        let chunks = random_chunks(&mut rng, &haystack);
        let chunked = ChunkedHaystack::new(&chunks);

        // Backtracking engine.
        if let Ok(re) = Regex::new(&pattern, flags) {
            let plain: Vec<(usize, usize)> =
                re.find_all(&haystack).map(|m| (m.start, m.end)).collect();
            let via_chunks: Vec<(usize, usize)> = re
                .find_all_from(chunked)
                .map(|m| (m.start, m.end))
                .collect();
            if plain != via_chunks {
                mismatches.push(format!(
                    "seed={seed} iter={i} engine=backtracking pattern={pattern:?} \
                     haystack={haystack:?} chunks={chunks:?}\n\
                     plain={plain:?}\nvia_chunks={via_chunks:?}"
                ));
            }
            checked += 1;
        }

        // Linear engine.
        if let Ok(re) = Regex::new_linear(&pattern, flags) {
            let plain: Vec<(usize, usize)> =
                re.find_all(&haystack).map(|m| (m.start, m.end)).collect();
            let via_chunks: Vec<(usize, usize)> = re
                .find_all_from(chunked)
                .map(|m| (m.start, m.end))
                .collect();
            if plain != via_chunks {
                mismatches.push(format!(
                    "seed={seed} iter={i} engine=linear pattern={pattern:?} \
                     haystack={haystack:?} chunks={chunks:?}\n\
                     plain={plain:?}\nvia_chunks={via_chunks:?}"
                ));
            }
            checked += 1;
        }
    }

    assert!(
        checked > iterations,
        "too many generated patterns were skipped as invalid (checked \
         {checked}/{} expected at least {iterations}) - grammar may be miscalibrated",
        iterations * 2
    );

    assert!(
        mismatches.is_empty(),
        "{} / {checked} chunked-vs-plain comparisons disagreed - first few:\n\n{}",
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
fn fuzz_oracle_streaming_chunking_is_transparent() {
    run_fuzz_iterations(0xC0FFEE, 800);
}

/// Heavier variant for manual/nightly-CI runs, mirroring the other fuzz
/// oracles.
#[test]
#[ignore]
fn fuzz_oracle_streaming_chunking_is_transparent_heavy() {
    run_fuzz_iterations(0xC0FFEE, 20_000);
}
