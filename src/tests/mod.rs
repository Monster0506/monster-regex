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
