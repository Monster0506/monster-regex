use crate::flags::Flags;
use crate::regex::Regex;

#[test]
fn test_linear_basic_match() {
    let re = Regex::new_linear("a+", Flags::default()).unwrap();
    assert!(re.is_match("aa"));
    assert!(!re.is_match("b"));
    assert_eq!(re.find("aa").unwrap().as_str("aa"), "aa");
}

#[test]
fn test_linear_captures() {
    let re = Regex::new_linear("(a+)b", Flags::default()).unwrap();
    let caps = re.captures("aaab").unwrap();
    assert_eq!(caps.get(0).unwrap().as_str("aaab"), "aaab");
    assert_eq!(caps.get(1).unwrap().as_str("aaab"), "aaa");
}

#[test]
fn test_linear_captures_multiple_groups() {
    let re = Regex::new_linear(r"(\w+)@(\w+)\.(\w+)", Flags::default()).unwrap();
    let caps = re.captures("user@example.com").unwrap();
    assert_eq!(
        caps.get(0).unwrap().as_str("user@example.com"),
        "user@example.com"
    );
    assert_eq!(caps.get(1).unwrap().as_str("user@example.com"), "user");
    assert_eq!(caps.get(2).unwrap().as_str("user@example.com"), "example");
    assert_eq!(caps.get(3).unwrap().as_str("user@example.com"), "com");
}

#[test]
fn test_linear_captures_named() {
    let re = Regex::new_linear(r"(?<year>\d{4})-(?<month>\d{2})", Flags::default()).unwrap();
    let caps = re.captures("2026-08").unwrap();
    assert_eq!(caps.as_str_named("2026-08", "year"), Some("2026"));
    assert_eq!(caps.as_str_named("2026-08", "month"), Some("08"));
}

#[test]
fn test_linear_captures_non_participating_group() {
    // The second alternative branch never runs, so group 1 shouldn't
    // participate in the match even though group 2 does.
    let re = Regex::new_linear(r"(foo)|(bar)", Flags::default()).unwrap();
    let caps = re.captures("bar").unwrap();
    assert!(caps.get(1).is_none());
    assert_eq!(caps.get(2).unwrap().as_str("bar"), "bar");
}

#[test]
fn test_linear_captures_repeated_group_last_iteration() {
    // A capture group inside a `+` should hold only the last iteration.
    let re = Regex::new_linear(r"(\d+,)+", Flags::default()).unwrap();
    let caps = re.captures("1,22,333,").unwrap();
    assert_eq!(caps.get(0).unwrap().as_str("1,22,333,"), "1,22,333,");
    assert_eq!(caps.get(1).unwrap().as_str("1,22,333,"), "333,");
}

#[test]
fn test_linear_captures_all() {
    let re = Regex::new_linear(r"(\w+)=(\w+)", Flags::default()).unwrap();
    let text = "a=1 b=2 c=3";
    let all: Vec<_> = re
        .captures_all(text)
        .map(|c| {
            (
                c.as_str(text, 1).unwrap().to_string(),
                c.as_str(text, 2).unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        all,
        vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
            ("c".to_string(), "3".to_string()),
        ]
    );
}

#[test]
fn test_linear_replace_with_capture_references() {
    let re = Regex::new_linear(
        r"(?<year>\d{4})-(?<month>\d{2})-(?<day>\d{2})",
        Flags::default(),
    )
    .unwrap();
    assert_eq!(
        re.replace("Date: 2026-08-29", "$month/$day/$year"),
        "Date: 08/29/2026"
    );
    assert_eq!(
        re.replace("Date: 2026-08-29", "${month}/${day}/${year}"),
        "Date: 08/29/2026"
    );
}

#[test]
fn test_linear_replace_non_participating_group() {
    let re = Regex::new_linear(r"(foo)|(bar)", Flags::default()).unwrap();
    assert_eq!(re.replace("bar", "[1=$1][2=$2]"), "[1=][2=bar]");
}

#[test]
fn test_fallback_api() {
    // Verify standard API still works (Backtracking engine default)
    let re = Regex::new("a+", Flags::default()).unwrap();
    assert!(re.is_match("aa"));
}

#[test]
fn test_linear_lazy_quantifier_prefers_minimum() {
    let re = Regex::new_linear("a??", Flags::default()).unwrap();
    let matches: Vec<_> = re.find_all("ab").map(|m| (m.start, m.end)).collect();
    assert_eq!(matches, vec![(0, 0), (1, 1), (2, 2)]);

    let re = Regex::new_linear("a*?", Flags::default()).unwrap();
    let matches: Vec<_> = re.find_all("ab").map(|m| (m.start, m.end)).collect();
    assert_eq!(matches, vec![(0, 0), (1, 1), (2, 2)]);
}

#[test]
fn test_linear_alternation_is_leftmost_first_not_longest() {
    let re = Regex::new_linear("a|aa", Flags::default()).unwrap();
    let matches: Vec<_> = re.find_all("aa").map(|m| (m.start, m.end)).collect();
    assert_eq!(matches, vec![(0, 1), (1, 2)]);
}

#[test]
fn test_linear_multi_literal_alternation_is_leftmost_first() {
    let re = Regex::new_linear("cat|category", Flags::default()).unwrap();
    let m = re.find("category").unwrap();
    assert_eq!(m.as_str("category"), "cat");
}

#[test]
fn test_linear_greedy_group_plus_repeats_fully() {
    let re = Regex::new_linear("(a)+", Flags::default()).unwrap();
    assert_eq!(re.find("aaa").unwrap().as_str("aaa"), "aaa");
    let caps = re.captures("aaa").unwrap();
    assert_eq!(caps.get(0).unwrap().as_str("aaa"), "aaa");

    let re = Regex::new_linear(r"(\d+,)+", Flags::default()).unwrap();
    let caps = re.captures("1,22,333,").unwrap();
    assert_eq!(caps.get(0).unwrap().as_str("1,22,333,"), "1,22,333,");
    assert_eq!(caps.get(1).unwrap().as_str("1,22,333,"), "333,");
}

#[test]
fn test_linear_bitparallel_overlapping_alphabet_stops_where_it_should() {
    let re = Regex::new_linear("[^ b-c1]*[^bbb]", Flags::default()).unwrap();
    let matches: Vec<_> = re.find_all("11a0a00").map(|m| (m.start, m.end)).collect();
    assert_eq!(matches, vec![(0, 1), (1, 2), (2, 7)]);
}

#[test]
fn test_linear_nested_quantifier_does_not_overshoot() {
    let re = Regex::new_linear(r"([b]{0}?[b-c]\d+\D{0,})+", Flags::default()).unwrap();
    let matches: Vec<_> = re
        .find_all("cc011 aa 111b1a ab1")
        .map(|m| (m.start, m.end))
        .collect();
    assert_eq!(matches, vec![(1, 9), (12, 18)]);
}
