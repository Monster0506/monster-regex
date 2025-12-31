/// Errors that can occur during the compilation of a regular expression.
#[derive(Debug)]
pub enum CompileError {
    /// The pattern contains invalid syntax.
    InvalidPattern(String),
    /// A quantifier (e.g., `*`, `+`, `{n,m}`) is used incorrectly or is invalid.
    InvalidQuantifier(String),
    /// A capture group is malformed.
    InvalidGroup(String),
    /// Parentheses are not balanced.
    UnmatchedParen,
    /// An escape sequence is invalid.
    InvalidEscape(String),
    /// A named capture group uses a name that has already been used.
    DuplicateGroupName(String),
}
