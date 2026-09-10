use super::{Flags, Regex};

fn assert_match(pattern: &str, text: &str) {
    let re = Regex::new(pattern, Flags::default())
        .unwrap_or_else(|e| panic!("Failed to compile pattern '{pattern}': {e:?}"));
    assert!(
        re.is_match(text),
        "Pattern '{pattern}' should match text '{text}'"
    );
}

fn assert_no_match(pattern: &str, text: &str) {
    let re = Regex::new(pattern, Flags::default())
        .unwrap_or_else(|e| panic!("Failed to compile pattern '{pattern}': {e:?}"));
    assert!(
        !re.is_match(text),
        "Pattern '{pattern}' should NOT match text '{text}'"
    );
}

fn assert_find(pattern: &str, text: &str, expected_match: &str) {
    let re = Regex::new(pattern, Flags::default())
        .unwrap_or_else(|e| panic!("Failed to compile pattern '{pattern}': {e:?}"));
    let m = re
        .find(text)
        .unwrap_or_else(|| panic!("Pattern '{pattern}' should find a match in '{text}'"));
    let matched_text = &text[m.start..m.end];
    assert_eq!(
        matched_text, expected_match,
        "Pattern '{pattern}' found '{matched_text}' but expected '{expected_match}'"
    );
}

#[test]
fn test_whole_pattern_recursion_balanced_parens() {
    // (?R) recurses the whole pattern - the textbook example: balanced
    // parentheses, which no plain (non-recursive) regex can recognize.
    let re = Regex::new(r"\((?:[^()]|(?R))*\)", Flags::default()).unwrap();

    assert!(re.is_match("()"));
    assert!(re.is_match("(a)"));
    assert!(re.is_match("(a(b)c)"));
    assert!(re.is_match("((x))"));
    assert!(re.is_match("(a(b(c)d)e)"));

    // Whole match must span the full balanced group, not stop early.
    let m = re.find("(a(b)c)").unwrap();
    assert_eq!(&"(a(b)c)"[m.start..m.end], "(a(b)c)");
}

#[test]
fn test_whole_pattern_recursion_and_anchors_interact() {
    let re = Regex::new(r"^\((?:[^()]|(?R))*\)$", Flags::default()).unwrap();
    assert!(re.is_match("()"));
    // "(a)" never actually needs the (?R) branch - "a" matches directly
    // via `[^()]` - so the anchor problem never comes into play here.
    assert!(re.is_match("(a)"));
    // "(a(b)c)" *does* need (?R) (for the nested "(b)"), where it hits the
    // anchor issue and fails.
    assert!(!re.is_match("(a(b)c)"));

    // Unanchored, the same recursive structure works at any depth - see
    // `test_whole_pattern_recursion_balanced_parens`.
    assert!(!re.is_match("(a(b)c"));
}

#[test]
fn test_named_recursion_rejects_unbalanced() {
    let re = Regex::new(r"^(?<paren>\((?:[^()]|(?&paren))*\))$", Flags::default()).unwrap();

    assert!(re.is_match("(a(b(c)d)e)"));
    assert!(!re.is_match("(a(b)c"));
    assert!(!re.is_match("a(b)c)"));
    assert!(!re.is_match("(a(b(c)d)"));
}

#[test]
fn test_balanced_parens_comprehensive() {
    let re = Regex::new(r"^(?<paren>\((?:[^()]|(?&paren))*\))$", Flags::default()).unwrap();

    let cases: &[(&str, bool)] = &[
        ("()", true),
        ("(a)", true),
        ("(a(b)c)", true),
        ("((x))", true),
        ("(a(b(c)d)e)", true),
        ("((()))", true),
        ("()()", false),
        ("(", false),
        (")", false),
        ("(()", false),
        ("())", false),
        ("(a(b)c", false),
        ("a(b)c)", false),
        ("", false),
        ("abc", false),
    ];
    for (input, expected) in cases {
        assert_eq!(
            re.is_match(input),
            *expected,
            "is_match({input:?}) should be {expected}"
        );
    }

    // Unanchored, find() locates a balanced group embedded in surrounding
    // content, spanning exactly the balanced part.
    let re_find = Regex::new(r"(?<paren>\((?:[^()]|(?&paren))*\))", Flags::default()).unwrap();
    let text = "prefix (a(b)c) suffix";
    let m = re_find.find(text).unwrap();
    assert_eq!(&text[m.start..m.end], "(a(b)c)");
}

#[test]
fn test_named_recursion_balanced_parens() {
    // (?<name>...) defines a group; (?&name) calls it. Same balanced-
    // parens capability as (?R), via an explicitly named self-reference.
    let re = Regex::new(r"^(?<paren>\((?:[^()]|(?&paren))*\))$", Flags::default()).unwrap();

    assert!(re.is_match("()"));
    assert!(re.is_match("(a(b)c)"));
    assert!(re.is_match("((x))"));
    assert!(!re.is_match("(a(b)c"));
}

#[test]
fn test_numbered_subroutine_call() {
    // (?1) calls group 1 - a fresh evaluation of its pattern, not a
    // backreference to its previous match.
    assert_match(r"([a-z]+)-(?1)", "hello-world");
    assert_find(r"([a-z]+)-(?1)", "hello-world", "hello-world");
}

#[test]
fn test_subroutine_call_is_not_a_backreference() {
    assert_match(r"([a-z]+)-\1", "hello-hello");
    assert_no_match(r"([a-z]+)-\1", "hello-world");

    assert_match(r"([a-z]+)-(?1)", "hello-hello");
    assert_match(r"([a-z]+)-(?1)", "hello-world");
}

#[test]
fn test_relative_subroutine_calls() {
    // (?-1): the group most recently opened before this call.
    assert_match(r"(a)(?-1)", "aa");
    assert_no_match(r"(a)(?-1)", "ab");

    // (?+1): the group opened immediately after this call.
    assert_match(r"(?:(?+1))(a)", "aa");
}

#[test]
fn test_p_greater_than_named_call_syntax() {
    // (?P>name) is PCRE's alternate spelling for (?&name).
    let re = Regex::new(r"^(?<paren>\((?:[^()]|(?P>paren))*\))$", Flags::default()).unwrap();
    assert!(re.is_match("(a(b)c)"));
    assert!(!re.is_match("(a(b)c"));
}

#[test]
fn test_mutual_recursion() {
    let re = Regex::new(r"^(?<A>a(?&B)?b)(?<B>c(?&A)?d)$", Flags::default()).unwrap();

    // Base case: both optional calls skipped - A = "ab", B = "cd".
    assert!(re.is_match("abcd"));

    assert!(re.is_match("acabdbcd"));

    assert!(!re.is_match("abc"));
}

#[test]
fn test_recursion_updates_call_site_capture() {
    let re = Regex::new(r"^(a)(?1)?$", Flags::default()).unwrap();
    let caps = re.captures("aa").unwrap();
    // Group 1 is defined at (0, 1) but the optional `(?1)` call re-matches
    // "a" at (1, 2) and rebinds group 1 to that span instead.
    let g1 = caps.get(1).expect("group 1 should have participated");
    assert_eq!((g1.start, g1.end), (1, 2));
}

#[test]
fn test_invalid_subroutine_reference_is_a_compile_error() {
    assert!(Regex::new(r"(?5)", Flags::default()).is_err());
    assert!(Regex::new(r"(a)(?2)", Flags::default()).is_err());
    assert!(Regex::new(r"(?&nonexistent)", Flags::default()).is_err());
    assert!(Regex::new(r"(a)(?-5)", Flags::default()).is_err());
}

#[test]
fn test_unbounded_recursion_fails_safely_not_stack_overflow() {
    let re = Regex::new(r"^(?<A>(?&A))$", Flags::default()).unwrap();
    assert!(!re.is_match("anything"));
    assert!(!re.is_match(""));
}

#[test]
fn test_bounded_deep_recursion_fails_safely_not_stack_overflow() {
    let re = Regex::new(r"^(?<paren>\((?:[^()]|(?&paren))*\))$", Flags::default()).unwrap();

    // Comfortably past the depth cap - must fail to match, not crash.
    let deep = "(".repeat(200) + &")".repeat(200);
    assert!(!re.is_match(&deep));

    // Comfortably within it - must still match correctly.
    let shallow = "(".repeat(15) + &")".repeat(15);
    assert!(re.is_match(&shallow));
}

#[test]
fn test_linear_engine_rejects_subroutine_calls() {
    assert!(Regex::new_linear(r"\((?:[^()]|(?R))*\)", Flags::default()).is_err());
    assert!(Regex::new_linear(r"([a-z]+)-(?1)", Flags::default()).is_err());
}
