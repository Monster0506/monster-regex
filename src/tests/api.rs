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

    assert!(re.is_match(text));
    assert!(re.find(text).is_some());
    let caps = re.captures(text).unwrap();
    assert_eq!(caps.get(0).unwrap().as_str(text), "abc");
    assert!(caps.groups.is_empty());

    let matches: Vec<_> = re.find_all(text).collect();
    assert!(matches.len() == 2);

    let captures: Vec<_> = re.captures_all(text).collect();
    assert_eq!(captures.len(), 2);

    assert_eq!(re.replace(text, "XYZ"), "XYZ def abc");
    assert_eq!(re.replace_all(text, "XYZ"), "XYZ def XYZ");
}

#[test]
fn test_flags_default() {
    let flags = Flags::default();
    assert_eq!(flags.ignore_case, None);
    assert!(!flags.multiline);
    assert!(!flags.dotall);
    assert!(!flags.verbose);
    assert!(!flags.unicode);
    assert!(!flags.global);
    assert_eq!(flags.max_backtrack_steps, None);
}

#[test]
fn test_regex_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Regex<crate::engine::backtracking::BacktrackingRegexEngine>>();
    assert_send_sync::<Regex<crate::engine::linear::LinearRegexEngine>>();
}

#[test]
fn test_linear_regex_concurrent_find_is_sound() {
    use std::sync::Arc;
    use std::thread;

    let re = Arc::new(Regex::new_linear(r"\w+@\w+\.\w+", Flags::default()).unwrap());
    let haystack: Arc<String> = Arc::new(
        (0..2000)
            .map(|i| format!("user{i}@example{i}.com "))
            .collect(),
    );
    let expected = re.find_all(haystack.as_str()).count();
    assert_eq!(expected, 2000);

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let re = Arc::clone(&re);
            let haystack = Arc::clone(&haystack);
            thread::spawn(move || re.find_all(haystack.as_str()).count())
        })
        .collect();

    for h in handles {
        assert_eq!(h.join().unwrap(), expected);
    }
}
