use crate::captures::{Captures, Match, expand_replacement};
use std::collections::HashMap;

#[test]
fn test_match_methods() {
    let m = Match { start: 5, end: 10 };
    let text = "0123456789012345";

    assert_eq!(m.len(), 5);
    assert!(!m.is_empty());
    assert_eq!(m.as_str(text), "56789");

    let empty_m = Match { start: 5, end: 5 };
    assert_eq!(empty_m.len(), 0);
    assert!(empty_m.is_empty());
    assert_eq!(empty_m.as_str(text), "");
}

#[test]
fn test_captures_access() {
    let text = "hello world";
    let full_match = Match { start: 0, end: 11 };
    let group1 = Match { start: 0, end: 5 }; // hello
    let group2 = Match { start: 6, end: 11 }; // world

    let mut named = HashMap::new();
    named.insert("greeting".to_string(), group1.clone());
    named.insert("object".to_string(), group2.clone());

    let captures = Captures {
        full_match: full_match.clone(),
        groups: vec![Some(group1.clone()), Some(group2.clone())],
        named,
    };

    // Test get()
    assert_eq!(captures.get(0), Some(&full_match));
    assert_eq!(captures.get(1), Some(&group1));
    assert_eq!(captures.get(2), Some(&group2));
    assert_eq!(captures.get(3), None);

    // Test get_named()
    assert_eq!(captures.get_named("greeting"), Some(&group1));
    assert_eq!(captures.get_named("object"), Some(&group2));
    assert_eq!(captures.get_named("verb"), None);

    // Test as_str()
    assert_eq!(captures.as_str(text, 0), Some("hello world"));
    assert_eq!(captures.as_str(text, 1), Some("hello"));
    assert_eq!(captures.as_str(text, 2), Some("world"));
    assert_eq!(captures.as_str(text, 3), None);

    // Test as_str_named()
    assert_eq!(captures.as_str_named(text, "greeting"), Some("hello"));
    assert_eq!(captures.as_str_named(text, "object"), Some("world"));
    assert_eq!(captures.as_str_named(text, "verb"), None);
}

#[test]
fn test_captures_optional_groups() {
    let text = "hello";
    let full_match = Match { start: 0, end: 5 };
    let group1 = Match { start: 0, end: 5 };

    let captures = Captures {
        full_match: full_match.clone(),
        groups: vec![Some(group1.clone()), None], // Second group didn't match
        named: HashMap::new(),
    };

    assert_eq!(captures.get(1), Some(&group1));
    assert_eq!(captures.get(2), None);

    assert_eq!(captures.as_str(text, 1), Some("hello"));
    assert_eq!(captures.as_str(text, 2), None);
}

fn caps_for_expand_tests() -> (Captures, &'static str) {
    let text = "user@example.com";
    let group1 = Match { start: 0, end: 4 }; // user
    let group2 = Match { start: 5, end: 12 }; // example
    let mut named = HashMap::new();
    named.insert("name".to_string(), group1.clone());
    named.insert("domain".to_string(), group2.clone());
    let captures = Captures {
        full_match: Match { start: 0, end: 16 },
        groups: vec![Some(group1), Some(group2)],
        named,
    };
    (captures, text)
}

#[test]
fn test_expand_replacement_numeric() {
    let (caps, text) = caps_for_expand_tests();
    assert_eq!(
        expand_replacement(&caps, "$1 at $2", text),
        "user at example"
    );
    assert_eq!(
        expand_replacement(&caps, "<$0>", text),
        "<user@example.com>"
    );
}

#[test]
fn test_expand_replacement_named() {
    let (caps, text) = caps_for_expand_tests();
    assert_eq!(
        expand_replacement(&caps, "$name @ $domain", text),
        "user @ example"
    );
    assert_eq!(
        expand_replacement(&caps, "${name}@${domain}.com", text),
        "user@example.com"
    );
}

#[test]
fn test_expand_replacement_literal_dollar() {
    let (caps, text) = caps_for_expand_tests();
    assert_eq!(expand_replacement(&caps, "$$$1", text), "$user");
    assert_eq!(
        expand_replacement(&caps, "no refs here", text),
        "no refs here"
    );
}

#[test]
fn test_expand_replacement_out_of_range_is_empty() {
    let (caps, text) = caps_for_expand_tests();
    assert_eq!(expand_replacement(&caps, "[$9]", text), "[]");
    assert_eq!(expand_replacement(&caps, "[$unknown]", text), "[]");
}

#[test]
fn test_expand_replacement_trailing_dollar() {
    let (caps, text) = caps_for_expand_tests();
    assert_eq!(expand_replacement(&caps, "total: $", text), "total: $");
}
