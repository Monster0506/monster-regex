use super::*;

#[cfg(test)]
#[path = "api.rs"]
mod api;

#[cfg(test)]
#[path = "rift_parsing.rs"]
mod rift_parsing;

#[cfg(test)]
#[path = "captures.rs"]
mod captures;

#[cfg(test)]
#[path = "regex_stubs.rs"]
mod regex_stubs;

#[cfg(test)]
#[path = "parser.rs"]
mod parser;

#[cfg(test)]
#[path = "engine.rs"]
mod engine;

#[cfg(test)]
#[path = "flags.rs"]
mod flags;

#[cfg(test)]
mod streaming;

#[cfg(test)]
#[path = "linear_api.rs"]
mod linear_api;

#[cfg(test)]
#[path = "fuzz_oracle.rs"]
mod fuzz_oracle;

#[cfg(test)]
#[path = "fuzz_oracle_lookaround.rs"]
mod fuzz_oracle_lookaround;

#[cfg(test)]
#[path = "fuzz_oracle_linear.rs"]
mod fuzz_oracle_linear;

#[cfg(test)]
#[path = "fuzz_oracle_streaming.rs"]
mod fuzz_oracle_streaming;

#[cfg(test)]
#[path = "subroutine.rs"]
mod subroutine;
