use crate::captures::{Captures, Match};
use crate::errors::CompileError;
use crate::flags::Flags;

// Iterators
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
        for group in &mut adjusted_caps.groups {
            if let Some(m) = group {
                m.start += offset;
                m.end += offset;
            }
        }
        for m in adjusted_caps.named.values_mut() {
            m.start += offset;
            m.end += offset;
        }

        Some(adjusted_caps)
    }
}

// Main regex type
pub struct Regex {
    pattern: String,
    flags: Flags,
    // Internal compiled representation
}

impl Regex {
    /// Compile a regex pattern with explicit flags
    pub fn new(pattern: &str, flags: Flags) -> Result<Self, CompileError> {
        // TODO: Validate and compile pattern
        Ok(Regex {
            pattern: pattern.to_string(),
            flags,
        })
    }

    /// Check if text matches anywhere
    pub fn is_match(&self, text: &str) -> bool {
        self.find(text).is_some()
    }

    /// Find first match
    pub fn find(&self, _text: &str) -> Option<Match> {
        // TODO: Implement matching
        None
    }

    /// Find all matches (lazy iterator)
    pub fn find_all<'a>(&'a self, text: &'a str) -> FindAllIterator<'a> {
        FindAllIterator {
            text,
            regex: self,
            last_end: 0,
        }
    }

    /// Get captures from first match
    pub fn captures(&self, _text: &str) -> Option<Captures> {
        // TODO: Implement with group extraction
        None
    }

    /// Get captures from all matches (lazy iterator)
    pub fn captures_all<'a>(&'a self, text: &'a str) -> CapturesIterator<'a> {
        CapturesIterator {
            text,
            regex: self,
            last_end: 0,
        }
    }

    /// Replace first match
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

    /// Replace all matches
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

    /// Access the original pattern
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Access the flags
    pub fn flags(&self) -> &Flags {
        &self.flags
    }
}
