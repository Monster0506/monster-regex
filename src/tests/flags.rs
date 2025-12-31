use crate::{Flags, Regex, parse_rift_format};

// Helper to assert flag parsing
fn assert_flags_parsed(pattern_with_flags: &str, expected_flags: Flags) {
    let (_, flags) = parse_rift_format(pattern_with_flags).expect("Failed to parse rift format");

    // Compare fields
    assert_eq!(
        flags.ignore_case, expected_flags.ignore_case,
        "ignore_case mismatch"
    );
    assert_eq!(
        flags.multiline, expected_flags.multiline,
        "multiline mismatch"
    );
    assert_eq!(flags.dotall, expected_flags.dotall, "dotall mismatch");
    assert_eq!(flags.verbose, expected_flags.verbose, "verbose mismatch");
    assert_eq!(flags.unicode, expected_flags.unicode, "unicode mismatch");
    assert_eq!(flags.global, expected_flags.global, "global mismatch");
}

#[test]
fn test_ignore_case_flag() {
    // 1. Parsing
    let mut expected = Flags::default();
    expected.ignore_case = Some(true);
    assert_flags_parsed("abc/i", expected);

    let mut expected_c = Flags::default();
    expected_c.ignore_case = Some(false);
    assert_flags_parsed("abc/c", expected_c);

    // 2. Behavior
    // Case Insensitive
    let re = Regex::new("abc", expected).unwrap();
    assert!(re.is_match("ABC"));
    assert!(re.is_match("aBc"));

    // Case Sensitive
    let re = Regex::new("abc", expected_c).unwrap();
    assert!(re.is_match("abc"));
    assert!(!re.is_match("ABC"));

    // Smartcase (default)
    // Lowercase pattern -> case insensitive
    let re = Regex::new("abc", Flags::default()).unwrap();
    assert!(re.is_match("ABC"));

    // Mixed case pattern -> case sensitive
    let re = Regex::new("Abc", Flags::default()).unwrap();
    assert!(!re.is_match("abc"));

    // Character class with ignore case
    let re = Regex::new("[a-z]", expected).unwrap();
    assert!(re.is_match("A"));
}

#[test]
fn test_multiline_flag() {
    // 1. Parsing
    let mut expected = Flags::default();
    expected.multiline = true;
    // parse_rift_format infers smartcase (Some(true) for "abc")
    expected.ignore_case = Some(true);
    assert_flags_parsed("abc/m", expected);

    // 2. Behavior
    let text = "foo\nbar\nbaz";

    // Without multiline
    let re = Regex::new("^bar", Flags::default()).unwrap();
    assert!(!re.is_match(text));

    // With multiline
    let mut flags = Flags::default();
    flags.multiline = true;
    let re = Regex::new("^bar", flags).unwrap();
    assert!(re.is_match(text));

    let re = Regex::new("bar$", flags).unwrap();
    assert!(re.is_match(text));
}

#[test]
fn test_dotall_flag() {
    // 1. Parsing
    let mut expected = Flags::default();
    expected.dotall = true;
    expected.ignore_case = Some(true);
    assert_flags_parsed("abc/s", expected);

    // 2. Behavior
    let text = "a\nb";

    // Without dotall
    let re = Regex::new("a.b", Flags::default()).unwrap();
    assert!(!re.is_match(text));

    // With dotall
    let mut flags = Flags::default();
    flags.dotall = true;
    let re = Regex::new("a.b", flags).unwrap();
    assert!(re.is_match(text));
}

#[test]
fn test_verbose_flag() {
    // 1. Parsing
    let mut expected = Flags::default();
    expected.verbose = true;
    expected.ignore_case = Some(true);
    assert_flags_parsed("abc/x", expected);

    // 2. Behavior
    let pattern = r"
        a  # match a
        b  # match b
        c
    ";

    // Without verbose (spaces and newlines are literal)
    let re = Regex::new(pattern, Flags::default()).unwrap();
    assert!(!re.is_match("abc"));

    // With verbose
    let mut flags = Flags::default();
    flags.verbose = true;
    let re = Regex::new(pattern, flags).unwrap();
    assert!(re.is_match("abc"));
}

#[test]
fn test_unicode_flag() {
    // 1. Parsing
    let mut expected = Flags::default();
    expected.unicode = true;
    expected.ignore_case = Some(true);
    assert_flags_parsed("abc/u", expected);

    // 2. Behavior
    let mut flags = Flags::default();
    flags.unicode = true;

    // \w matching unicode
    let re = Regex::new(r"\w", flags).unwrap();
    assert!(re.is_match("ü"));
    assert!(re.is_match("ñ"));
}

#[test]
fn test_global_flag() {
    // 1. Parsing
    let mut expected = Flags::default();
    expected.global = true;
    expected.ignore_case = Some(true);
    assert_flags_parsed("abc/g", expected);

    // 2. Behavior
    // The global flag is preserved in the struct
    let mut flags = Flags::default();
    flags.global = true;
    let re = Regex::new("a", flags).unwrap();
    assert!(re.flags().global);
}

#[test]
fn test_combined_flags() {
    let (pattern, flags) = parse_rift_format("abc/gims").unwrap();
    assert_eq!(pattern, "abc");
    assert!(flags.global);
    assert_eq!(flags.ignore_case, Some(true));
    assert!(flags.multiline);
    assert!(flags.dotall);
}
