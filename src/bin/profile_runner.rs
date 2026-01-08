use monster_regex::{Flags, Regex};
use std::time::Instant;

fn run_profile(name: &str, pattern: &str, input: &str) {
    println!("--- Profiling: {} ---", name);
    println!("Pattern: \"{}\"", pattern);
    let input_display = if input.len() > 50 {
        format!("{}... (len={})", &input[..50], input.len())
    } else {
        format!("\"{}\"", input)
    };
    println!("Input: {}", input_display);

    // Linear
    if let Ok(re) = Regex::new_linear(pattern, Flags::default()) {
        println!("\n[Linear Engine]");
        let start = Instant::now();
        let count = re.find_all(input).count();
        let dur = start.elapsed();
        println!("Matches: {}", count);
        println!("Time: {:.4}ms", dur.as_secs_f64() * 1000.0);
    } else {
        println!("\n[Linear Engine] Compile Failed");
    }

    // Backtracking
    if let Ok(re) = Regex::new(pattern, Flags::default()) {
        println!("\n[Backtracking Engine]");
        let start = Instant::now();
        let count = re.find_all(input).count();
        let dur = start.elapsed();
        println!("Matches: {}", count);
        println!("Time: {:.4}ms", dur.as_secs_f64() * 1000.0);
    } else {
        println!("\n[Backtracking Engine] Compile Failed");
    }
    println!("--------------------------------------------------\n");
}

fn generate_dna(len: usize) -> String {
    let chars = ['A', 'C', 'G', 'T'];
    (0..len).map(|i| chars[i % 4]).collect()
}

fn main() {
    println!("=== Monster Regex Profiling Suite ===");
    println!(
        "To see internal metrics, run with: cargo run --features internal_metrics --bin profile_runner\n"
    );

    // 1. Literals
    run_profile("Literal Short", "foo", "foo bar baz foo");
    run_profile(
        "Literal Long",
        "supercalifragilistic",
        &"supercalifragilisticexpialidocious".repeat(100),
    );

    // 2. Character Classes
    run_profile("Class Digit", r"\d+", "Item 1, Item 20, Item 300");
    run_profile("Class Custom", "[a-c]", "abracadabra");

    // 3. Anchors
    run_profile("Anchor Start", "^Hello", "Hello World\nHello Rust");
    run_profile("Anchor End", "End$", "Start to End\nJust End");
    run_profile("Word Boundary", r"\bword\b", "word sword password word");

    // 4. Quantifiers
    run_profile("Quantifier Star", "a*", "baaaab");
    run_profile("Quantifier Plus", "a+", "baaaab");
    run_profile("Quantifier Question", "a?", "baaaab");
    run_profile("Quantifier Range", "a{2,4}", "aa aaa aaaa aaaaa");

    // 5. Alternation
    run_profile("Alternation Simple", "cat|dog", "I have a cat and a dog.");
    run_profile("Alternation Overlap", "cal|calendar", "calendar calculator");

    // 6. Groups
    run_profile("Group Capture", r"(a)b", "ab ac ad ab");

    // 7. Complex/Case Studies
    let dna = generate_dna(10_000);
    run_profile("DNA Complex", r"[AC]+G+[TA]+", &dna);

    // 8. Pathological
    run_profile("Pathological (a+)+$", r"(a+)+$", "aaaaaaaaaaaaaaaaaaaa");
}
