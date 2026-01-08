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
fn test_fallback_api() {
    // Verify standard API still works (Backtracking engine default)
    let re = Regex::new("a+", Flags::default()).unwrap();
    assert!(re.is_match("aa"));
}
