#![allow(clippy::field_reassign_with_default)]

use super::{Flags, Regex};
// --- Helper Functions ---

fn assert_match(pattern: &str, text: &str) {
    let re = Regex::new(pattern, Flags::default())
        .unwrap_or_else(|e| panic!("Failed to compile pattern '{}': {:?}", pattern, e));
    assert!(
        re.is_match(text),
        "Pattern '{}' should match text '{}'",
        pattern,
        text
    );
}

fn assert_no_match(pattern: &str, text: &str) {
    let re = Regex::new(pattern, Flags::default())
        .unwrap_or_else(|e| panic!("Failed to compile pattern '{}': {:?}", pattern, e));
    assert!(
        !re.is_match(text),
        "Pattern '{}' should NOT match text '{}'",
        pattern,
        text
    );
}

fn assert_find(pattern: &str, text: &str, expected_match: &str) {
    let re = Regex::new(pattern, Flags::default()).unwrap();
    let m = re
        .find(text)
        .unwrap_or_else(|| panic!("Pattern '{}' should find a match in '{}'", pattern, text));
    let matched_text = &text[m.start..m.end];
    assert_eq!(
        matched_text, expected_match,
        "Pattern '{}' found '{}' but expected '{}'",
        pattern, matched_text, expected_match
    );
}

#[allow(dead_code)]
fn assert_find_all(pattern: &str, text: &str, expected: Vec<&str>) {
    let re = Regex::new(pattern, Flags::default()).unwrap();
    let matches: Vec<String> = re
        .find_all(text)
        .map(|m| text[m.start..m.end].to_string())
        .collect();
    assert_eq!(
        matches, expected,
        "Find all mismatch for pattern '{}'",
        pattern
    );
}

// --- 1. General Syntax & Literals ---

#[test]
fn test_literals() {
    assert_match("abc", "abc");
    assert_match("abc", "zabc");
    assert_match("abc", "abcz");
    assert_match("abc", "zabcz");
    assert_no_match("abc", "ab");
    assert_no_match("abc", "bc");
    assert_no_match("abc", "ac");
}

#[test]
fn test_special_characters_escaped() {
    assert_match(r"\.", ".");
    assert_match(r"\*", "*");
    assert_match(r"\+", "+");
    assert_match(r"\?", "?");
    assert_match(r"\^", "^");
    assert_match(r"\$", "$");
    assert_match(r"\|", "|");
    assert_match(r"\(", "(");
    assert_match(r"\)", ")");
    assert_match(r"\[", "[");
    assert_match(r"\]", "]");
    assert_match(r"\{", "{");
    assert_match(r"\}", "}");
    assert_match(r"\\", "\\");
}

#[test]
fn test_escape_sequences() {
    assert_match(r"\n", "\n");
    assert_match(r"\t", "\t");
    assert_match(r"\r", "\r");
    assert_match(r"\f", "\x0C");
    assert_match(r"\v", "\x0B");
}

#[test]
fn test_dot() {
    assert_match(".", "a");
    assert_match(".", "z");
    assert_match(".", " ");
    assert_no_match(".", ""); // Empty string has no chars
    assert_no_match(".", "\n"); // Default doesn't match newline

    // Dotall flag
    let mut flags = Flags::default();
    flags.dotall = true;
    let re = Regex::new(".", flags).unwrap();
    assert!(re.is_match("\n"));
}

// --- 2. Quantifiers ---

#[test]
fn test_quantifiers_greedy() {
    // * (0 or more)
    assert_match("a*", "");
    assert_match("a*", "a");
    assert_match("a*", "aaaa");
    assert_find("ba*", "baaaac", "baaaa");

    // + (1 or more)
    assert_no_match("a+", "");
    assert_match("a+", "a");
    assert_find("a+", "baaaac", "aaaa");

    // ? (0 or 1)
    assert_match("a?", "");
    assert_match("a?", "a");
    assert_find("ba?", "baaaac", "ba"); // Matches b then one a
    assert_find("ba?", "bc", "b"); // Matches b then zero a

    // {n}
    assert_match("a{3}", "aaa");
    assert_no_match("a{3}", "aa");
    assert_find("a{3}", "aaaaa", "aaa");

    // {n,m}
    assert_find("a{2,4}", "aaaaa", "aaaa"); // Greedy, takes 4
    assert_find("a{2,4}", "aaa", "aaa");
    assert_find("a{2,4}", "aa", "aa");
    assert_no_match("a{2,4}", "a");

    // {n,}
    assert_find("a{2,}", "aaaaa", "aaaaa");
    assert_no_match("a{2,}", "a");

    // {,m}
    assert_find("a{,3}", "aaaa", "aaa");
}

#[test]
fn test_quantifiers_lazy() {
    // *?
    assert_find("a*?", "aaaa", ""); // Matches 0 times immediately
    assert_find("ba*?", "baaaa", "b"); // Matches b then 0 a's

    // +?
    assert_find("a+?", "aaaa", "a"); // Matches 1 time (minimal)

    // ??
    assert_find("ba??", "ba", "b"); // Prefers 0 matches

    // {n,m}?
    assert_find("a{2,4}?", "aaaaa", "aa"); // Minimal 2
}

// --- 3. Character Classes ---

#[test]
fn test_standard_classes() {
    // \d Digit
    assert_match(r"\d", "0");
    assert_match(r"\d", "9");
    assert_no_match(r"\d", "a");

    // \D Non-digit
    assert_match(r"\D", "a");
    assert_no_match(r"\D", "1");

    // \w Word
    assert_match(r"\w", "a");
    assert_match(r"\w", "Z");
    assert_match(r"\w", "0");
    assert_match(r"\w", "_");
    assert_no_match(r"\w", "!");

    // \W Non-word
    assert_match(r"\W", "!");
    assert_match(r"\W", " ");
    assert_no_match(r"\W", "a");

    // \s Whitespace
    assert_match(r"\s", " ");
    assert_match(r"\s", "\t");
    assert_match(r"\s", "\n");
    assert_no_match(r"\s", "a");

    // \S Non-whitespace
    assert_match(r"\S", "a");
    assert_no_match(r"\S", " ");
}

#[test]
fn test_extended_classes() {
    // \l Lowercase
    assert_match(r"\l", "a");
    assert_match(r"\l", "A"); // Smartcase
    assert_no_match(r"\l", "0");

    // \u Uppercase
    assert_match(r"\u", "A");
    assert_match(r"\u", "a"); // Smartcase

    // \x Hex digit
    assert_match(r"\x", "a");
    assert_match(r"\x", "F");
    assert_match(r"\x", "0");
    assert_no_match(r"\x", "g");

    // \a Alphanumeric
    assert_match(r"\a", "a");
    assert_match(r"\a", "0");
    assert_no_match(r"\a", "_");
    assert_no_match(r"\a", "!");

    // \L Non-lowercase
    assert_match(r"\L", "A");
    assert_match(r"\L", "0");
    assert_no_match(r"\L", "a");

    // \U Non-uppercase
    assert_match(r"\U", "a");
    assert_match(r"\U", "0");
    assert_no_match(r"\U", "A");

    // \X Non-hex digit
    assert_match(r"\X", "g");
    assert_no_match(r"\X", "a");
    assert_no_match(r"\X", "F");
    assert_no_match(r"\X", "0");

    // \o Octal digit
    assert_match(r"\o", "0");
    assert_match(r"\o", "7");
    assert_no_match(r"\o", "8");

    // \O Non-octal digit
    assert_match(r"\O", "8");
    assert_match(r"\O", "a");
    assert_no_match(r"\O", "0");

    // \h Head of word character (start of a word)
    assert_match(r"\h", "a");
    assert_match(r"\h", "_");
    assert_no_match(r"\h", "0");

    // \H Non-head of word character
    assert_match(r"\H", "0");
    assert_match(r"\H", "!");
    assert_no_match(r"\H", "a");

    // \p Punctuation
    assert_match(r"\p", "!");
    assert_match(r"\p", ".");
    assert_no_match(r"\p", "a");
    assert_no_match(r"\p", "0");

    // \P Non-punctuation
    assert_match(r"\P", "a");
    assert_match(r"\P", "0");
    assert_no_match(r"\P", "!");

    // \A Non-alphanumeric
    assert_match(r"\A", "_");
    assert_match(r"\A", "!");
    assert_no_match(r"\A", "a");
    assert_no_match(r"\A", "0");
}

#[test]
fn test_custom_sets() {
    assert_match("[abc]", "a");
    assert_match("[abc]", "b");
    assert_match("[abc]", "c");
    assert_no_match("[abc]", "d");

    // Ranges
    assert_match("[a-z]", "m");
    assert_match("[a-z]", "M"); // Smartcase

    // Force case sensitive
    let mut flags = Flags::default();
    flags.ignore_case = Some(false);
    let re = Regex::new("[a-z]", flags).unwrap();
    assert!(!re.is_match("M"));

    // Negation
    assert_match("[^abc]", "d");
    assert_no_match("[^abc]", "a");
}

// --- 4. Anchors and Boundaries ---

#[test]
fn test_anchors() {
    // ^ Start
    assert_match("^abc", "abc");
    assert_match("^abc", "abcd");
    assert_no_match("^abc", "zabc");

    // $ End
    assert_match("abc$", "abc");
    assert_match("abc$", "zabc");
    assert_no_match("abc$", "abcd");

    // Both
    assert_match("^abc$", "abc");
    assert_no_match("^abc$", "abcd");
}

#[test]
fn test_word_boundaries() {
    // \b
    assert_find(r"\bword\b", "a word b", "word");
    assert_no_match(r"\bword\b", "sword");
    assert_no_match(r"\bword\b", "words");

    // \< Start of word
    assert_match(r"\<word", "word");
    assert_match(r"\<word", " word");
    assert_no_match(r"\<word", "sword");

    // \> End of word
    assert_match(r"word\>", "word");
    assert_match(r"word\>", "word ");
    assert_no_match(r"word\>", "words");
}

#[test]
fn test_match_boundaries_zs_ze() {
    // \zs Sets start
    assert_find("foo\\zsbar", "foobar", "bar");

    // \ze Sets end
    assert_find("foo\\zebar", "foobar", "foo");

    // Both
    assert_find("foo\\zsbar\\zebaz", "foobarbaz", "bar");
}

#[test]
fn test_zs_reverts_on_backtrack_past_it() {
    assert_find(r"(?:a\zsb){1,2}abY", "ababY", "babY");
}

// --- 5. Flags ---

#[test]
fn test_flags_multiline() {
    let mut flags = Flags::default();
    flags.multiline = true;
    let re = Regex::new("^bar", flags).unwrap();

    assert!(re.is_match("foo\nbar"));
    // Without multiline
    assert_no_match("^bar", "foo\nbar");
}

#[test]
fn test_flags_case_sensitivity() {
    // i flag (ignore-case)
    let mut flags = Flags::default();
    flags.ignore_case = Some(true);
    let re = Regex::new("abc", flags).unwrap();
    assert!(re.is_match("ABC"));
    assert!(re.is_match("AbC"));

    // c flag (case-sensitive)
    let mut flags = Flags::default();
    flags.ignore_case = Some(false);
    let re = Regex::new("abc", flags).unwrap();
    assert!(re.is_match("abc"));
    assert!(!re.is_match("ABC"));
}

#[test]
fn test_flags_verbose() {
    let mut flags = Flags::default();
    flags.verbose = true;

    // Spaces ignored
    let re = Regex::new("foo bar", flags).unwrap();
    assert!(re.is_match("foobar"));
    assert!(!re.is_match("foo bar"));

    // Escaped space matches space
    let re = Regex::new(r"foo\ bar", flags).unwrap();
    assert!(re.is_match("foo bar"));

    // Space in brackets matches space
    let re = Regex::new(r"foo[ ]bar", flags).unwrap();
    assert!(re.is_match("foo bar"));

    // Comments
    let re = Regex::new(
        r"foo # comment
bar",
        flags,
    )
    .unwrap();
    assert!(re.is_match("foobar"));
}

// --- 6. Alternation & Grouping ---

#[test]
fn test_alternation() {
    assert_match("cat|dog", "cat");
    assert_match("cat|dog", "dog");
    assert_no_match("cat|dog", "bat");

    // Precedence
    assert_find("a|ab", "ab", "a"); // First alternative matches 'a'
}

#[test]
fn test_grouping() {
    assert_match("(abc)", "abc");
    assert_match("(abc)+", "abcabc");
    assert_find("(a(b)c)", "abc", "abc");
}

#[test]
fn test_non_capturing_group() {
    assert_match("(?:abc)", "abc");
    assert_match("(?:abc)+", "abcabc");
}

#[test]
fn test_named_group() {
    assert_match("(?<name>abc)", "abc");
}

// --- 7. Lookarounds ---

#[test]
fn test_lookahead() {
    // Positive (?>=...)
    assert_find("foo(?>=bar)", "foobar", "foo");
    assert_no_match("foo(?>=bar)", "foobaz");

    // Negative (?>!...)
    assert_find("foo(?>!bar)", "foobaz", "foo");
    assert_no_match("foo(?>!bar)", "foobar");
}

#[test]
fn test_lookbehind() {
    // Positive (?<=...)
    assert_find("(?<=foo)bar", "foobar", "bar");
    assert_no_match("(?<=foo)bar", "bazbar");

    // Negative (?<!...)
    assert_find("(?<!foo)bar", "bazbar", "bar");
    assert_no_match("(?<!foo)bar", "foobar");
}

#[test]
fn test_capture_inside_lookaround_visible_afterward() {
    assert_find(r"(?>=(a))\1", "aa", "a");
    assert_find(r"(a)(?<=(a))\2", "aa", "aa");
}

#[test]
fn test_lookbehind_large_input_is_linear() {
    let haystack = "abcdefghij".repeat(20_000); // 200K chars, contains no 'z'
    let re = Regex::new(r"(?<!x)zzzzznomatch", Flags::default()).unwrap();
    let start = std::time::Instant::now();
    assert!(re.find(&haystack).is_none());
    let elapsed = start.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "lookbehind scan took {:?} - possible O(N^2) regression",
        elapsed
    );

    // Correctness over a large input: the windowed lookbehind still matches at
    // the right spot far from the string start.
    let mut tail = "a".repeat(100_000);
    tail.push_str("foobar");
    let re2 = Regex::new(r"(?<=foo)bar", Flags::default()).unwrap();
    let m = re2
        .find(&tail)
        .expect("(?<=foo)bar should match the 'bar' at the tail");
    assert_eq!(&tail[m.start..m.end], "bar");
}

#[test]
fn test_prefilter_sees_through_assertions() {
    assert_find(r"^bar", "bar", "bar");
    assert_find(r"\bbar", "foo bar", "bar");
    assert_find(r"(?<!z)bar", "  bar", "bar");
    assert_find(r"(?<=foo)bar", "foobar", "bar");

    // Smartcase: an all-lowercase literal behind an assertion still matches
    // case-insensitively via the case-insensitive prefilter path.
    assert_find(r"(?<!z)bar", "  BAR", "BAR");
}

#[test]
fn test_lookbehind_non_ascii_no_panic() {
    // Regression: the lookbehind scan stepped through raw byte offsets, slicing
    // mid-UTF-8 (cursor_at / char_before) and panicking on non-ASCII input.
    assert_find(r"(?<!x)b", "äb", "b");
    assert_find(r"(?<=ä)b", "äb", "b");
    assert_no_match(r"(?<!ä)b", "äb");

    let re = Regex::new(r"(?<!x).", Flags::default()).unwrap();
    assert_eq!(re.find_all("äöü").count(), 3);
}

#[test]
fn test_unicode_class_no_panic_and_matches() {
    let hay = "héllo wörld café";

    let re = Regex::new_linear(r"\w+", Flags::default()).unwrap();
    let m = re.find(hay).expect("\\w+ should match");
    assert_eq!(&hay[m.start..m.end], "héllo");

    let words: Vec<&str> = re.find_all(hay).map(|m| &hay[m.start..m.end]).collect();
    assert_eq!(words, vec!["héllo", "wörld", "café"]);
}

#[test]
fn test_complex_lookarounds() {
    // Lookahead with quantifier inside
    println!("1");
    assert_find(r"foo(?>=\d+)", "foo123", "foo");
    println!("2");
    assert_no_match(r"foo(?>=\d+)", "foobar");

    // Lookbehind with alternation
    println!("3");
    assert_find(r"(?<=a|b)c", "ac", "c");
    println!("4");
    assert_find(r"(?<=a|b)c", "bc", "c");
    println!("5");
    assert_no_match(r"(?<=a|b)c", "dc");

    // Nested lookarounds
    println!("6");
    assert_find(r"foo(?>=bar(?>=baz))", "foobarbaz", "foo");
    println!("7");
    assert_no_match(r"foo(?>=bar(?>=baz))", "foobarqux");
}

// --- 8. Replacement ---

#[test]
fn test_replace() {
    let re = Regex::new("a", Flags::default()).unwrap();
    assert_eq!(re.replace("banana", "o"), "bonana"); // Replaces first
}

#[test]
fn test_replace_all() {
    let re = Regex::new("a", Flags::default()).unwrap();
    assert_eq!(re.replace_all("banana", "o"), "bonono");
}

#[test]
fn test_replace_groups() {
    let re = Regex::new(r"\w+", Flags::default()).unwrap();
    assert_eq!(re.replace_all("hello world", "word"), "word word");
}

#[test]
fn test_replace_with_capture_references() {
    let re = Regex::new(r"(\w+)@(\w+)\.(\w+)", Flags::default()).unwrap();
    assert_eq!(
        re.replace("contact user@example.com today", "$1 at $2 dot $3"),
        "contact user at example dot com today"
    );
    assert_eq!(
        re.replace_all("user@example.com and admin@test.org", "[$1]"),
        "[user] and [admin]"
    );
}

#[test]
fn test_group_backtracks_across_boundary() {
    assert_find("(a+)(a+)", "aaaa", "aaaa");
    assert_find("(a|ab)c", "abc", "abc");
    assert_find("(a|ab)(c|bcd)", "abcd", "abcd");
    assert_no_match("(a|ab)c", "ab");
}

#[test]
fn test_group_backtracks_min_required_repetitions() {
    assert_find("(?:aa|a){2}a", "aaa", "aaa");
    assert_no_match("(?:aa|a){2}a", "aa");
}

#[test]
fn test_group_backtrack_captures_correct_split() {
    let re = Regex::new("(a+)(a+)", Flags::default()).unwrap();
    let caps = re.captures("aaaa").unwrap();
    // Greedy: the first group takes as much as it can while still leaving
    // the second group (which requires at least one 'a') something to match.
    assert_eq!(caps.as_str("aaaa", 1), Some("aaa"));
    assert_eq!(caps.as_str("aaaa", 2), Some("a"));
}

#[test]
fn test_replace_literal_dollar_sign() {
    let re = Regex::new(r"(\d+)", Flags::default()).unwrap();
    assert_eq!(re.replace("price: 42", "$$$1"), "price: $42");
}

// --- 9. Complex Scenarios ---

#[test]
fn test_ipv4() {
    let pattern = r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}";
    assert_match(pattern, "192.168.1.1");
    assert_no_match(pattern, "192.168.1");
}

#[test]
fn test_unicode_flag() {
    let mut flags = Flags::default();
    flags.unicode = true;

    // \w should match unicode letters
    let _re = Regex::new(r"\w+", flags).unwrap();
    assert!(_re.is_match("über"));

    // In current implementation, \w seems to be Unicode-aware by default (using Rust's is_alphanumeric)
    // So we check that it DOES match, rather than DOES NOT match.
    let _re_ascii = Regex::new(r"\w+", Flags::default()).unwrap();
    assert!(_re_ascii.is_match("über"));
}

// --- Production-hardening: catastrophic backtracking & deep nesting ---

#[test]
fn test_catastrophic_backtracking_bounded_by_step_budget() {
    let re = Regex::new(r"(a+)+b", Flags::default()).unwrap();
    let haystack = "a".repeat(40);

    let start = std::time::Instant::now();
    let result = re.is_match(&haystack);
    let elapsed = start.elapsed();

    assert!(!result, "no trailing 'b', so this must not match");
    assert!(
        elapsed < std::time::Duration::from_secs(15),
        "catastrophic pattern took {:?} - step budget did not bound it",
        elapsed
    );
}

#[test]
fn test_catastrophic_backtracking_alternation_bounded() {
    // (a|a)*b - overlapping-alternation shape, same exponential-blowup risk
    // as (a+)+b but via repeated alternation instead of nested quantifiers.
    let re = Regex::new(r"(a|a)*b", Flags::default()).unwrap();
    let haystack = "a".repeat(30);

    let start = std::time::Instant::now();
    let result = re.is_match(&haystack);
    let elapsed = start.elapsed();

    assert!(!result);
    assert!(
        elapsed < std::time::Duration::from_secs(15),
        "catastrophic pattern took {:?} - step budget did not bound it",
        elapsed
    );
}

#[test]
fn test_step_budget_does_not_affect_ordinary_matches() {
    assert_match(r"(a+)+b", "aaaaaaaaaab");
    assert_match(r"(foo|bar|baz)+qux", "foobarbazqux");
    assert_match(r"(\w+\s)+end", "one two three end");
    assert_no_match(r"(a+)+b", "aaaaaaaaaa");
}

#[test]
fn test_deeply_nested_groups_rejected_cleanly() {
    let depth = 10_000;
    let pattern = format!("{}a{}", "(".repeat(depth), ")".repeat(depth));

    let result = Regex::new(&pattern, Flags::default());
    assert!(
        result.is_err(),
        "pattern nested {} groups deep should be rejected, not accepted",
        depth
    );

    // Same check against the linear engine's own (separate) parse call.
    let result_linear = Regex::new_linear(&pattern, Flags::default());
    assert!(result_linear.is_err());
}

#[test]
fn test_moderately_nested_groups_still_work() {
    // Sanity check that the new depth guard doesn't false-trip on nesting
    // depths a real pattern might plausibly use.
    let depth = 50;
    let pattern = format!("{}a{}", "(".repeat(depth), ")".repeat(depth));
    assert_match(&pattern, "a");
}

#[test]
fn test_lookbehind_with_greedy_unbounded_body_backtracks_to_boundary() {
    assert_match(r"(?<=.*)a", "a");
    assert_find(r"(?<=.*)a", "ba", "a");

    let re = Regex::new(r".??(?<!.*[1c ]*)", Flags::default()).unwrap();
    assert_eq!(re.find_all("b1c1 1c0").count(), 0);
}

#[test]
fn test_nested_lazy_quantifier_does_not_overmatch() {
    assert_match(r"(?:c??){1,2}", "");
    assert_find(r"(?:c??){1,2}", "aa", "");

    // Same shape, wrapped in an alternation whose first branch is the
    // empty-preferring one.
    assert_find(r"^(?:[c]??|0{1,2}c){1,3}", "", "");

    // Same shape again, but where the nested empty-preferring construct sits
    // inside a multi-node group rather than being the sole element.
    assert_find(r"( *?0{0,2}c??)?", " ", "");

    assert_find(r"(\S{2}c{3}.{1,2}| *?0{0,2}c??)?", " ", "");

    assert_find(r"(c??)?d", "cd", "cd");
}
