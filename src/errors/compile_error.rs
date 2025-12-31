#[derive(Debug)]
pub enum CompileError {
    InvalidPattern(String),
    InvalidQuantifier(String),
    InvalidGroup(String),
    UnmatchedParen,
    InvalidEscape(String),
    DuplicateGroupName(String),
}
