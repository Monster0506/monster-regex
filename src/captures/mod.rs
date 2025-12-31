/// Represents a single match within the text, defined by a start and end byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// The byte index where the match starts (inclusive).
    pub start: usize,
    /// The byte index where the match ends (exclusive).
    pub end: usize,
}

impl Match {
    /// Returns the length of the match in bytes.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Returns true if the match has a length of 0.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Returns the substring of the original text corresponding to this match.
    ///
    /// # Panics
    ///
    /// Panics if the indices are out of bounds of the provided text or do not lie on UTF-8 boundaries.
    pub fn as_str<'a>(&self, text: &'a str) -> &'a str {
        &text[self.start..self.end]
    }
}

/// Represents the results of a regex match, including the full match and any captured groups.
#[derive(Debug, Clone)]
pub struct Captures {
    /// The match corresponding to the entire regex pattern (group 0).
    pub full_match: Match,
    /// Ordered list of captured groups (group 1, group 2, etc.).
    /// `None` indicates the group exists in the pattern but did not participate in the match.
    pub groups: Vec<Option<Match>>,
    /// Map of named capture groups to their matches.
    pub named: std::collections::HashMap<String, Match>,
}

impl Captures {
    /// Returns the match associated with the capture group at `index`.
    ///
    /// * `0` corresponds to the entire match.
    /// * `1..` corresponds to the parenthesized capture groups.
    ///
    /// Returns `None` if the index is out of bounds or if the group did not participate in the match.
    pub fn get(&self, index: usize) -> Option<&Match> {
        if index == 0 {
            Some(&self.full_match)
        } else {
            self.groups.get(index - 1).and_then(|g| g.as_ref())
        }
    }

    /// Returns the match associated with a named capture group.
    pub fn get_named(&self, name: &str) -> Option<&Match> {
        self.named.get(name)
    }

    /// Returns the substring of the original text for the capture group at `index`.
    pub fn as_str<'a>(&self, text: &'a str, index: usize) -> Option<&'a str> {
        self.get(index).map(|m| m.as_str(text))
    }

    /// Returns the substring of the original text for a named capture group.
    pub fn as_str_named<'a>(&self, text: &'a str, name: &str) -> Option<&'a str> {
        self.get_named(name).map(|m| m.as_str(text))
    }
}
