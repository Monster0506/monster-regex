use super::*;

#[test]
fn test_parse_rift_format() {
    let (pattern, flags) = parse_rift_format("foo/i").unwrap();
    assert_eq!(pattern, "foo");
    assert_eq!(flags.ignore_case, Some(true));

    let (pattern, flags) = parse_rift_format("Foo/").unwrap();
    assert_eq!(pattern, "Foo");
    assert_eq!(flags.ignore_case, Some(false)); // has uppercase
}

#[test]
fn test_parse_rift_format_complex() {
    // Multiple flags
    let (pattern, flags) = parse_rift_format("abc/gm").unwrap();
    assert_eq!(pattern, "abc");
    assert!(flags.global);
    assert!(flags.multiline);
    assert_eq!(flags.ignore_case, Some(true)); // inferred smartcase (lowercase)

    // Smartcase inference
    let (pattern, flags) = parse_rift_format("abc/").unwrap();
    assert_eq!(pattern, "abc");
    assert_eq!(flags.ignore_case, Some(true)); // lowercase -> ignore case

    let (pattern, flags) = parse_rift_format("Abc/").unwrap();
    assert_eq!(pattern, "Abc");
    assert_eq!(flags.ignore_case, Some(false)); // uppercase -> case sensitive

    // Explicit case overrides smartcase
    let (pattern, flags) = parse_rift_format("abc/c").unwrap();
    assert_eq!(pattern, "abc");
    assert_eq!(flags.ignore_case, Some(false));

    let (pattern, flags) = parse_rift_format("Abc/i").unwrap();
    assert_eq!(pattern, "Abc");
    assert_eq!(flags.ignore_case, Some(true));
}

#[test]
fn test_parse_rift_format_special_chars() {
    // Pattern with slashes
    let (pattern, flags) = parse_rift_format("foo/bar/i").unwrap();
    assert_eq!(pattern, "foo/bar");
    assert_eq!(flags.ignore_case, Some(true));
}

#[test]
fn test_parse_rift_format_errors() {
    // Missing delimiter
    assert!(matches!(
        parse_rift_format("foo"),
        Err(ParseError::NoDelimiter)
    ));

    // Invalid flag
    assert!(matches!(
        parse_rift_format("foo/z"),
        Err(ParseError::InvalidFlags('z'))
    ));
}
