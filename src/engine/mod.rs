pub mod backtracking;
pub mod linear;
use crate::captures::{Captures, Match};
use crate::errors::CompileError;
use crate::flags::Flags;
use crate::haystack::Haystack;
pub use backtracking::Matcher;

pub trait RegexEngine: Send + Sync + 'static {
    /// The compiled regex type produced by this engine.
    type Regex: CompiledRegex;

    /// Compile a regex pattern using this engine.
    fn compile(&self, pattern: &str, flags: Flags) -> Result<Self::Regex, CompileError>;
}

pub trait CompiledRegex: Send + Sync {
    /// Returns the original pattern string.
    fn pattern(&self) -> &str;

    /// Returns the flags used during compilation.
    fn flags(&self) -> &Flags;

    /// Returns true if the regex matches anywhere in the text.
    fn is_match(&self, text: &str) -> bool;

    /// Finds the first match in the text.
    fn find(&self, text: &str) -> Option<Match>;

    /// Returns an iterator over all non-overlapping matches in the text.
    fn find_all<'a>(&'a self, text: &'a str) -> Box<dyn Iterator<Item = Match> + 'a>;

    /// Returns the first match and its capture groups.
    fn captures(&self, text: &str) -> Option<Captures>;

    /// Returns an iterator yielding capture groups for each match.
    fn captures_all<'a>(&'a self, text: &'a str) -> Box<dyn Iterator<Item = Captures> + 'a>;

    /// Replace the first match with the replacement string.
    fn replace(&self, text: &str, replacement: &str) -> String;

    /// Replace all non-overlapping matches with the replacement string.
    fn replace_all(&self, text: &str, replacement: &str) -> String;
}

impl CompiledRegex for Box<dyn CompiledRegex> {
    fn pattern(&self) -> &str {
        (**self).pattern()
    }

    fn flags(&self) -> &Flags {
        (**self).flags()
    }

    fn is_match(&self, text: &str) -> bool {
        (**self).is_match(text)
    }

    fn find(&self, text: &str) -> Option<Match> {
        (**self).find(text)
    }

    fn find_all<'a>(&'a self, text: &'a str) -> Box<dyn Iterator<Item = Match> + 'a> {
        (**self).find_all(text)
    }

    fn captures(&self, text: &str) -> Option<Captures> {
        (**self).captures(text)
    }

    fn captures_all<'a>(&'a self, text: &'a str) -> Box<dyn Iterator<Item = Captures> + 'a> {
        (**self).captures_all(text)
    }

    fn replace(&self, text: &str, replacement: &str) -> String {
        (**self).replace(text, replacement)
    }

    fn replace_all(&self, text: &str, replacement: &str) -> String {
        (**self).replace_all(text, replacement)
    }
}

pub trait CompiledRegexHaystack: CompiledRegex {
    /// Returns true if the regex matches anywhere in the haystack.
    fn is_match_from<H: Haystack>(&self, haystack: H) -> bool;

    /// Finds the first match in the haystack.
    fn find_from<H: Haystack>(&self, haystack: H) -> Option<Match>;

    /// Finds the first match starting at the given byte offset.
    fn find_from_at<H: Haystack>(&self, haystack: H, start: usize) -> Option<Match>;

    /// Returns an iterator over all non-overlapping matches in the haystack.
    fn find_all_from<'a, H: Haystack + 'a>(
        &'a self,
        haystack: H,
    ) -> Box<dyn Iterator<Item = Match> + 'a>;
}

pub struct AnyRegexEngine<E: RegexEngine>(pub E);

impl<E: RegexEngine> RegexEngine for AnyRegexEngine<E>
where
    E::Regex: 'static,
{
    type Regex = Box<dyn CompiledRegex>;

    fn compile(&self, pattern: &str, flags: Flags) -> Result<Self::Regex, CompileError> {
        let regex = self.0.compile(pattern, flags)?;
        Ok(Box::new(regex))
    }
}
