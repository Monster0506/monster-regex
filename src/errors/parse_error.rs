#[derive(Debug)]
pub enum ParseError {
    NoDelimiter,
    InvalidFlags(char),
}
