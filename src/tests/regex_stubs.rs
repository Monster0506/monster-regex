use crate::{Flags, Regex};

#[test]
fn test_stub_find() {
    let re = Regex::new("abc", Flags::default()).unwrap();
    assert!(re.find("abc").is_some());
    assert!(re.is_match("abc"));
}

#[test]
fn test_stub_captures() {
    let re = Regex::new("abc", Flags::default()).unwrap();
    // No capture groups in the pattern, but "abc" matches - captures()
    // reports the full match with an empty group list.
    let caps = re.captures("abc").unwrap();
    assert_eq!(caps.get(0).unwrap().as_str("abc"), "abc");
    assert!(caps.groups.is_empty());
}

#[test]
fn test_stub_replace() {
    let re = Regex::new("abc", Flags::default()).unwrap();
    let text = "abc def";
    assert_eq!(re.replace(text, "xyz"), "xyz def");
}

#[test]
fn test_stub_replace_all() {
    let re = Regex::new("abc", Flags::default()).unwrap();
    let text = "xyz def xyz";
    assert_eq!(re.replace_all(text, "xyz"), text);
}

#[test]
fn test_stub_iterators() {
    let re = Regex::new("abc", Flags::default()).unwrap();
    let text = "abc def abc";

    let matches: Vec<_> = re.find_all(text).collect();
    assert!(matches.len() == 2);

    let captures: Vec<_> = re.captures_all(text).collect();
    assert_eq!(captures.len(), 2);
}
