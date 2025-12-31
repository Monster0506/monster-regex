use crate::captures::{Captures, Match};
use crate::errors::CompileError;
use crate::flags::Flags;

/// An iterator over all non-overlapping matches of a regex in a string.
///
/// Yields `Match` objects.
pub struct FindAllIterator<'a> {
    text: &'a str,
    regex: &'a Regex,
    last_end: usize,
}

impl<'a> Iterator for FindAllIterator<'a> {
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
///
/// Yields `Captures` objects.
pub struct CapturesIterator<'a> {
    text: &'a str,
    regex: &'a Regex,
    last_end: usize,
}

impl<'a> Iterator for CapturesIterator<'a> {
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

/// A compiled regular expression.
///
/// This struct represents a parsed and compiled regex pattern, ready to be used for matching against text.
pub struct Regex {
    pattern: String,
    flags: Flags,
    // Internal compiled representation
}

impl Regex {
    /// Compiles a regex pattern with the specified flags.
    ///
    /// # Arguments
    ///
    /// * `pattern` - The regex pattern string.
    /// * `flags` - Configuration flags for the regex engine.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the compiled `Regex` or a `CompileError` if the pattern is invalid.
    pub fn new(pattern: &str, flags: Flags) -> Result<Self, CompileError> {
        // TODO: Validate and compile pattern
        Ok(Regex {
            pattern: pattern.to_string(),
            flags,
        })
    }

    /// Checks if the regex matches anywhere in the given text.
    ///
    /// Returns `true` if a match is found, `false` otherwise.
    pub fn is_match(&self, text: &str) -> bool {
        self.find(text).is_some()
    }

    /// Finds the first occurrence of the regex in the text.
    ///
    /// Returns `Some(Match)` if a match is found, or `None` otherwise.
    pub fn find(&self, _text: &str) -> Option<Match> {
        // TODO: Implement matching
        None
    }

    /// Returns an iterator over all non-overlapping matches in the text.
    pub fn find_all<'a>(&'a self, text: &'a str) -> FindAllIterator<'a> {
        FindAllIterator {
            text,
            regex: self,
            last_end: 0,
        }
    }

    /// Finds the first match and returns the capture groups.
    ///
    /// Returns `Some(Captures)` if a match is found, containing the full match and any captured groups.
    /// Returns `None` if no match is found.
    pub fn captures(&self, _text: &str) -> Option<Captures> {
        // TODO: Implement with group extraction
        None
    }

    /// Returns an iterator over all non-overlapping matches, yielding capture groups for each match.
    pub fn captures_all<'a>(&'a self, text: &'a str) -> CapturesIterator<'a> {
        CapturesIterator {
            text,
            regex: self,
            last_end: 0,
        }
    }

    /// Replaces the first match in the text with the replacement string.
    ///
    /// If no match is found, returns the original text.
    pub fn replace(&self, text: &str, replacement: &str) -> String {
        if let Some(m) = self.find(text) {
            let mut result = String::with_capacity(text.len());
            result.push_str(&text[..m.start]);
            result.push_str(replacement);
            result.push_str(&text[m.end..]);
            result
        } else {
            text.to_string()
        }
    }

    /// Replaces all non-overlapping matches in the text with the replacement string.
    pub fn replace_all(&self, text: &str, replacement: &str) -> String {
        let mut result = String::with_capacity(text.len() * 2);
        let mut last_end = 0;

        for m in self.find_all(text) {
            result.push_str(&text[last_end..m.start]);
            result.push_str(replacement);
            last_end = m.end;
        }

        result.push_str(&text[last_end..]);
        result
    }

    /// Returns the original pattern string used to compile this regex.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Returns the flags used to compile this regex.
    pub fn flags(&self) -> &Flags {
        &self.flags
    }
}
