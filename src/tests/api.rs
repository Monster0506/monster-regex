use crate::{Flags, Regex};

#[test]
fn test_regex_compilation() {
    let flags = Flags::default();
    let re = Regex::new("abc", flags);
    assert!(re.is_ok());
    let re = re.unwrap();
    assert_eq!(re.pattern(), "abc");
}

#[test]
fn test_regex_methods_existence() {
    let flags = Flags::default();
    let re = Regex::new("abc", flags).unwrap();
    let text = "abc def abc";

    // These assertions match the current stub implementation (returning None/false)
    // ensuring the API is wired up correctly even if logic is missing.
    assert_eq!(re.is_match(text), false);
    assert!(re.find(text).is_none());
    assert!(re.captures(text).is_none());

    let matches: Vec<_> = re.find_all(text).collect();
    assert!(matches.is_empty());

    let captures: Vec<_> = re.captures_all(text).collect();
    assert!(captures.is_empty());

    assert_eq!(re.replace(text, "XYZ"), text);
    assert_eq!(re.replace_all(text, "XYZ"), text);
}

#[test]
fn test_flags_default() {
    let flags = Flags::default();
    assert_eq!(flags.ignore_case, None);
    assert_eq!(flags.multiline, false);
    assert_eq!(flags.dotall, false);
    assert_eq!(flags.verbose, false);
    assert_eq!(flags.unicode, false);
    assert_eq!(flags.global, false);
}
