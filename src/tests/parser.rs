use super::*;

#[test]
fn test_literal() {
    let mut p = Parser::new("abc", Flags::default());
    let ast = p.parse().unwrap();
    assert_eq!(ast.len(), 3);
}

#[test]
fn test_dot() {
    let mut p = Parser::new("a.c", Flags::default());
    let ast = p.parse().unwrap();
    assert_eq!(ast.len(), 3);
    assert!(matches!(ast[1], AstNode::CharClass(CharClass::Dot)));
}

#[test]
fn test_quantifiers() {
    let mut p = Parser::new("a+", Flags::default());
    let ast = p.parse().unwrap();
    assert_eq!(ast.len(), 1);
    assert!(matches!(ast[0], AstNode::OneOrMore { .. }));
}

#[test]
fn test_chained_quantifier() {
    let mut p = Parser::new(".{2}{0}", Flags::default());
    let ast = p.parse().unwrap();
    assert_eq!(ast.len(), 1, "should collapse into one quantified node, not a node plus leftover literals");
    assert!(matches!(ast[0], AstNode::Exact { count: 0, .. }));
}

#[test]
fn test_char_class() {
    let mut p = Parser::new("[a-z]", Flags::default());
    let ast = p.parse().unwrap();
    assert_eq!(ast.len(), 1);
}

#[test]
fn test_group() {
    let mut p = Parser::new("(abc)", Flags::default());
    let ast = p.parse().unwrap();
    assert_eq!(ast.len(), 1);
    assert!(matches!(ast[0], AstNode::Group { .. }));
}

#[test]
fn test_escape_classes() {
    let mut p = Parser::new(r"\d\w\s", Flags::default());
    let ast = p.parse().unwrap();
    assert_eq!(ast.len(), 3);
}

#[test]
fn test_lookarounds() {
    // Positive lookahead (?>=...)
    let mut p = Parser::new("(?>=abc)", Flags::default());
    let ast = p.parse().unwrap();
    assert!(matches!(ast[0], AstNode::LookAhead { positive: true, .. }));

    // Negative lookahead (?>!...)
    let mut p = Parser::new("(?>!abc)", Flags::default());
    let ast = p.parse().unwrap();
    assert!(matches!(
        ast[0],
        AstNode::LookAhead {
            positive: false,
            ..
        }
    ));

    // Positive lookbehind (?<=...)
    let mut p = Parser::new("(?<=abc)", Flags::default());
    let ast = p.parse().unwrap();
    assert!(matches!(ast[0], AstNode::LookBehind { positive: true, .. }));

    // Negative lookbehind (?<!...)
    let mut p = Parser::new("(?<!abc)", Flags::default());
    let ast = p.parse().unwrap();
    assert!(matches!(
        ast[0],
        AstNode::LookBehind {
            positive: false,
            ..
        }
    ));
}
