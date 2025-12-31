use crate::captures::{Captures, Match};
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
