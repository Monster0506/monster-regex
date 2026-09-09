use crate::{Flags, Regex};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

const ALPHABET: &[char] = &['a', 'b', 'c', '0', '1', ' '];
const META: &str = "\\^$.|?*+()[]{}";

struct Gen {
    rng: StdRng,
    group_count: usize,
}

impl Gen {
    fn new(seed: u64) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            group_count: 0,
        }
    }

    fn literal(&mut self) -> String {
        let c = ALPHABET[self.rng.gen_range(0..ALPHABET.len())];
        if META.contains(c) {
            format!("\\{c}")
        } else {
            c.to_string()
        }
    }

    fn class(&mut self) -> &'static str {
        match self.rng.gen_range(0..6) {
            0 => r"\d",
            1 => r"\D",
            2 => r"\w",
            3 => r"\W",
            4 => r"\s",
            _ => r"\S",
        }
    }

    fn set(&mut self) -> String {
        let n = self.rng.gen_range(1..=3);
        let mut s = String::from("[");
        if self.rng.gen_bool(0.3) {
            s.push('^');
        }
        for _ in 0..n {
            if self.rng.gen_bool(0.3) {
                let lo = b'a' + self.rng.gen_range(0..3);
                let hi = lo + self.rng.gen_range(0..3);
                s.push(lo as char);
                s.push('-');
                s.push(hi as char);
            } else {
                s.push(ALPHABET[self.rng.gen_range(0..ALPHABET.len())]);
            }
        }
        s.push(']');
        s
    }

    fn quantifier(&mut self) -> String {
        let q = match self.rng.gen_range(0..7) {
            0 => String::new(),
            1 => "*".to_string(),
            2 => "+".to_string(),
            3 => "?".to_string(),
            4 => format!("{{{}}}", self.rng.gen_range(0..=3)),
            5 => format!("{{{},}}", self.rng.gen_range(0..=2)),
            _ => {
                let m = self.rng.gen_range(0..=2);
                let n = m + self.rng.gen_range(0..=2);
                format!("{{{m},{n}}}")
            }
        };
        if !q.is_empty() && self.rng.gen_bool(0.3) {
            format!("{q}?")
        } else {
            q
        }
    }

    fn atom(&mut self, depth: u32, allow_lookaround: bool) -> (String, String, bool) {
        if depth == 0 {
            let lit = self.literal();
            return (lit.clone(), lit, false);
        }
        let choice = if allow_lookaround {
            self.rng.gen_range(0..9)
        } else {
            match self.rng.gen_range(0..6) {
                5 => 8,
                n => n,
            }
        };
        match choice {
            0 => {
                let lit = self.literal();
                (lit.clone(), lit, false)
            }
            1 => {
                let c = self.class().to_string();
                (c.clone(), c, false)
            }
            2 => {
                let s = self.set();
                (s.clone(), s, false)
            }
            3 => (".".to_string(), ".".to_string(), false),
            4 => {
                let (mbody, fbody) = self.alternation(depth - 1, allow_lookaround);
                if self.rng.gen_bool(0.5) {
                    self.group_count += 1;
                    (format!("({mbody})"), format!("({fbody})"), false)
                } else {
                    (format!("(?:{mbody})"), format!("(?:{fbody})"), false)
                }
            }
            5 | 6 => {
                let (mbody, fbody) = self.alternation(depth - 1, false);
                if self.rng.gen_bool(0.5) {
                    (format!("(?>={mbody})"), format!("(?={fbody})"), true)
                } else {
                    (format!("(?>!{mbody})"), format!("(?!{fbody})"), true)
                }
            }
            7 => {
                let (mbody, fbody) = self.alternation(depth - 1, false);
                if self.rng.gen_bool(0.5) {
                    (format!("(?<={mbody})"), format!("(?<={fbody})"), true)
                } else {
                    (format!("(?<!{mbody})"), format!("(?<!{fbody})"), true)
                }
            }
            _ => {
                // Backreference to an already-closed group, if one exists;
                // otherwise fall back to a literal.
                if self.group_count > 0 {
                    let idx = self.rng.gen_range(1..=self.group_count);
                    let s = format!("\\{idx}");
                    (s.clone(), s, false)
                } else {
                    let lit = self.literal();
                    (lit.clone(), lit, false)
                }
            }
        }
    }

    fn sequence(&mut self, depth: u32, allow_lookaround: bool) -> (String, String) {
        let n = self.rng.gen_range(1..=3);
        let mut m = String::new();
        let mut f = String::new();
        for _ in 0..n {
            let (matom, fatom, is_lookaround) = self.atom(depth, allow_lookaround);
            m.push_str(&matom);
            f.push_str(&fatom);
            if !is_lookaround {
                let q = self.quantifier();
                m.push_str(&q);
                f.push_str(&q);
            }
        }
        (m, f)
    }

    fn alternation(&mut self, depth: u32, allow_lookaround: bool) -> (String, String) {
        let n = self.rng.gen_range(1..=2);
        let mut mbranches = Vec::new();
        let mut fbranches = Vec::new();
        for _ in 0..n {
            let (m, f) = self.sequence(depth, allow_lookaround);
            mbranches.push(m);
            fbranches.push(f);
        }
        (mbranches.join("|"), fbranches.join("|"))
    }

    fn pattern(&mut self) -> (String, String) {
        self.group_count = 0;
        let (mut m, mut f) = self.alternation(2, true);
        if self.rng.gen_bool(0.15) {
            m = format!("^{m}");
            f = format!("^{f}");
        }
        if self.rng.gen_bool(0.15) {
            m = format!("{m}$");
            f = format!("{f}$");
        }
        (m, f)
    }

    fn haystack(&mut self) -> String {
        let len = self.rng.gen_range(0..=25);
        (0..len)
            .map(|_| ALPHABET[self.rng.gen_range(0..ALPHABET.len())])
            .collect()
    }
}

fn fancy_oracle_for(pattern: &str) -> Option<fancy_regex::Regex> {
    let case_insensitive = !pattern.chars().any(|c| c.is_uppercase());
    fancy_regex::RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .backtrack_limit(30_000)
        .build()
        .ok()
}

fn run_fuzz_iterations(seed: u64, iterations: u32, known_pre_existing_mismatches: usize) {
    let mut g = Gen::new(seed);
    let mut checked = 0u32;

    let flags = Flags {
        max_backtrack_steps: Some(30_000),
        ..Flags::default()
    };
    // See `fuzz_oracle.rs`: only used to re-check an apparent mismatch
    // before counting it as one, so no impact on the common case's speed.
    let big_budget_flags = Flags {
        max_backtrack_steps: Some(50_000_000),
        ..Flags::default()
    };

    let mut mismatches: Vec<String> = Vec::new();

    let mut monster_fail = 0u32;
    let mut fancy_fail = 0u32;
    let mut monster_fail_example: Option<String> = None;
    let mut fancy_fail_example: Option<(String, String)> = None;

    for i in 0..iterations {
        let (mpattern, fpattern) = g.pattern();
        let haystack = g.haystack();

        let monster_res = Regex::new(&mpattern, flags);
        if monster_res.is_err() && monster_fail_example.is_none() {
            monster_fail_example = Some(mpattern.clone());
        }
        if monster_res.is_err() {
            monster_fail += 1;
        }
        let Ok(monster) = monster_res else {
            continue;
        };
        let fancy_res = fancy_oracle_for(&fpattern);
        if fancy_res.is_none() {
            fancy_fail += 1;
            if fancy_fail_example.is_none() {
                fancy_fail_example = Some((mpattern.clone(), fpattern.clone()));
            }
        }
        let Some(oracle) = fancy_res else {
            continue;
        };

        let monster_matches: Vec<(usize, usize)> = monster
            .find_all(&haystack)
            .map(|m| (m.start, m.end))
            .collect();

        let mut oracle_matches = Vec::new();
        let mut oracle_errored = false;
        for m in oracle.find_iter(&haystack) {
            match m {
                Ok(m) => oracle_matches.push((m.start(), m.end())),
                Err(_) => {
                    oracle_errored = true;
                    break;
                }
            }
        }
        if oracle_errored {
            continue;
        }

        if monster_matches != oracle_matches {
            let monster_big: Vec<(usize, usize)> = Regex::new(&mpattern, big_budget_flags)
                .unwrap()
                .find_all(&haystack)
                .map(|m| (m.start, m.end))
                .collect();
            if monster_big != oracle_matches {
                mismatches.push(format!(
                    "seed={seed} iter={i}\nmonster_pattern={mpattern:?}\nfancy_pattern={fpattern:?}\n\
                     haystack={haystack:?}\nmonster={monster_matches:?}\nfancy_regex={oracle_matches:?}"
                ));
            }
        }
        checked += 1;
    }

    assert!(
        checked > iterations / 3,
        "too many generated patterns were skipped as invalid on one side or \
         the other (checked {checked}/{iterations}, monster_fail={monster_fail} \
         fancy_fail={fancy_fail}) - grammar may be miscalibrated\n\
         monster_fail_example={monster_fail_example:?}\nfancy_fail_example={fancy_fail_example:?}"
    );

    assert!(
        mismatches.len() <= known_pre_existing_mismatches,
        "{} / {checked} generated cases disagreed with the fancy-regex oracle \
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
fn fuzz_oracle_lookaround_vs_fancy_regex() {
    run_fuzz_iterations(0xC0FFEE, 800, 2);
}

#[test]
#[ignore]
fn fuzz_oracle_lookaround_vs_fancy_regex_heavy() {
    run_fuzz_iterations(0xC0FFEE, 20_000, 14);
}
