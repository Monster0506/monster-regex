use crate::captures::{Captures, Match};
use crate::engine::backtracking::BacktrackingRegexEngine;
use crate::engine::linear::LinearRegexEngine;
use crate::engine::{CompiledRegex, CompiledRegexHaystack, RegexEngine};
use crate::errors::CompileError;
use crate::flags::Flags;
use crate::haystack::Haystack;
// use crate::parser::AstNode;
// BacktrackingRegex exposes ast, compiled linear regex exposes nfa.
// Regex struct just holds compiled.

/// A compiled regular expression.
///
/// This struct represents a parsed and compiled regex pattern, ready to be used for matching against text.
pub struct Regex<E: RegexEngine = BacktrackingRegexEngine> {
    compiled: E::Regex,
}

impl Regex<BacktrackingRegexEngine> {
    /// Compiles a regex pattern with the specified flags using the default Backtracking engine.
    pub fn new(pattern: &str, flags: Flags) -> Result<Self, CompileError> {
        let engine = BacktrackingRegexEngine;
        let compiled = engine.compile(pattern, flags)?;
        Ok(Regex { compiled })
    }
}

impl Regex<LinearRegexEngine> {
    /// Compiles a regex pattern with the specified flags using the Linear engine.
    pub fn new_linear(pattern: &str, flags: Flags) -> Result<Self, CompileError> {
        let engine = LinearRegexEngine;
        let compiled = engine.compile(pattern, flags)?;
        Ok(Regex { compiled })
    }
}

impl<E: RegexEngine> Regex<E> {
    /// Checks if the regex matches anywhere in the given text.
    pub fn is_match(&self, text: &str) -> bool {
        self.compiled.is_match(text)
    }

    /// Checks if the regex matches anywhere in the given haystack.
    pub fn is_match_from<H: Haystack>(&self, text: H) -> bool
    where
        E::Regex: crate::engine::CompiledRegexHaystack,
    {
        self.compiled.is_match_from(text)
    }

    /// Finds the first occurrence of the regex in the text.
    pub fn find(&self, text: &str) -> Option<Match> {
        self.compiled.find(text)
    }

    /// Finds the first occurrence of the regex in the haystack.
    pub fn find_from<H: Haystack>(&self, text: H) -> Option<Match>
    where
        E::Regex: crate::engine::CompiledRegexHaystack,
    {
        self.compiled.find_from(text)
    }

    /// Finds the first occurrence of the regex in the haystack starting at the given position.
    pub fn find_from_at<H: Haystack>(&self, text: H, start: usize) -> Option<Match>
    where
        E::Regex: crate::engine::CompiledRegexHaystack,
    {
        self.compiled.find_from_at(text, start)
    }

    /// Returns an iterator over all non-overlapping matches in the text.
    pub fn find_all<'a>(&'a self, text: &'a str) -> FindAllIterator<'a, E> {
        FindAllIterator {
            text,
            regex: self,
            last_end: 0,
        }
    }

    /// Returns an iterator over all non-overlapping matches in the haystack.
    pub fn find_all_from<'a, H: Haystack>(&'a self, text: H) -> FindMatchesIterator<'a, H, E>
    where
        E::Regex: crate::engine::CompiledRegexHaystack,
    {
        FindMatchesIterator {
            text,
            regex: self,
            last_end: 0,
        }
    }

    /// Finds the first match and returns the capture groups.
    pub fn captures(&self, text: &str) -> Option<Captures> {
        self.compiled.captures(text)
    }

    /// Returns an iterator yielding capture groups for each match.
    pub fn captures_all<'a>(&'a self, text: &'a str) -> CapturesIterator<'a, E> {
        CapturesIterator {
            text,
            regex: self,
            last_end: 0,
        }
    }

    /// Replaces the first match in the text with the replacement string.
    pub fn replace(&self, text: &str, replacement: &str) -> String {
        self.compiled.replace(text, replacement)
    }

    /// Replaces all non-overlapping matches in the text with the replacement string.
    pub fn replace_all(&self, text: &str, replacement: &str) -> String {
        self.compiled.replace_all(text, replacement)
    }

    /// Returns the original pattern string used to compile this regex.
    pub fn pattern(&self) -> &str {
        self.compiled.pattern()
    }

    /// Returns the flags used to compile this regex.
    pub fn flags(&self) -> &Flags {
        self.compiled.flags()
    }
}

/// An iterator over all non-overlapping matches of a regex in a haystack.
pub struct FindMatchesIterator<'a, H: Haystack, E: RegexEngine> {
    text: H,
    regex: &'a Regex<E>,
    last_end: usize,
}

impl<'a, H: Haystack, E: RegexEngine> Iterator for FindMatchesIterator<'a, H, E>
where
    E::Regex: crate::engine::CompiledRegexHaystack,
{
    type Item = Match;

    fn next(&mut self) -> Option<Self::Item> {
        if self.last_end > self.text.len() {
            return None;
        }
        let m = self.regex.find_from_at(self.text.clone(), self.last_end)?;
        self.last_end = m.end.max(m.start + 1);
        Some(m)
    }
}

/// An iterator over all non-overlapping matches of a regex in a string.
pub struct FindAllIterator<'a, E: RegexEngine> {
    text: &'a str,
    regex: &'a Regex<E>,
    last_end: usize,
}

impl<'a, E: RegexEngine> Iterator for FindAllIterator<'a, E> {
    type Item = Match;

    fn next(&mut self) -> Option<Self::Item> {
        if self.last_end > self.text.len() {
            return None;
        }
        let m = self.regex.find(&self.text[self.last_end..])?;
        let adjusted = Match {
            start: self.last_end + m.start,
            end: self.last_end + m.end,
        };
        self.last_end = adjusted.end.max(adjusted.start + 1);
        Some(adjusted)
    }
}

/// An iterator over all non-overlapping capture groups of a regex in a string.
pub struct CapturesIterator<'a, E: RegexEngine> {
    text: &'a str,
    regex: &'a Regex<E>,
    last_end: usize,
}

impl<'a, E: RegexEngine> Iterator for CapturesIterator<'a, E> {
    type Item = Captures;

    fn next(&mut self) -> Option<Self::Item> {
        if self.last_end > self.text.len() {
            return None;
        }
        let caps = self.regex.captures(&self.text[self.last_end..])?;
        let offset = self.last_end;
        self.last_end = offset + caps.full_match.end;
        self.last_end = self.last_end.max(offset + caps.full_match.start + 1);

        // Adjust all match positions by offset
        let mut adjusted_caps = caps;
        adjusted_caps.full_match.start += offset;
        adjusted_caps.full_match.end += offset;
        for m in &mut adjusted_caps.groups.iter_mut().flatten() {
            m.start += offset;
            m.end += offset;
        }
        for m in adjusted_caps.named.values_mut() {
            m.start += offset;
            m.end += offset;
        }

        Some(adjusted_caps)
    }
}
