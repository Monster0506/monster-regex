pub mod captures;
pub mod errors;
pub mod flags;
pub mod parser;
pub mod parsing;
pub mod regex;

pub use captures::{Captures, Match};
pub use errors::{CompileError, ParseError};
pub use flags::Flags;
pub use parser::{AstNode, CharClass, CharRange, Parser};
pub use parsing::parse_rift_format;
pub use regex::Regex;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
