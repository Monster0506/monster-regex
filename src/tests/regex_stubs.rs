use crate::{Flags, Regex};

#[test]
fn test_stub_find() {
    let re = Regex::new("abc", Flags::default()).unwrap();
    assert!(re.find("abc").is_none());
    assert!(!re.is_match("abc"));
}

#[test]
fn test_stub_captures() {
    let re = Regex::new("abc", Flags::default()).unwrap();
    assert!(re.captures("abc").is_none());
}

#[test]
fn test_stub_replace() {
    let re = Regex::new("abc", Flags::default()).unwrap();
    let text = "abc def";
    assert_eq!(re.replace(text, "xyz"), text);
}

#[test]
fn test_stub_replace_all() {
    let re = Regex::new("abc", Flags::default()).unwrap();
    let text = "abc def abc";
    assert_eq!(re.replace_all(text, "xyz"), text);
}

#[test]
fn test_stub_iterators() {
    let re = Regex::new("abc", Flags::default()).unwrap();
    let text = "abc def abc";

    let matches: Vec<_> = re.find_all(text).collect();
    assert!(matches.is_empty());

    let captures: Vec<_> = re.captures_all(text).collect();
    assert!(captures.is_empty());
}
