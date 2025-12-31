/// Errors that can occur when parsing a Rift-formatted regex string (e.g., "pattern/flags").
#[derive(Debug)]
pub enum ParseError {
    /// The input string does not contain the expected delimiter (usually `/`).
    NoDelimiter,
    /// An invalid flag character was encountered.
    InvalidFlags(char),
}
