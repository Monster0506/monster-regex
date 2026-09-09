#[derive(Default)]
pub(crate) struct AdjacentEmptyFilter {
    prev_nonempty_end: Option<usize>,
}

impl AdjacentEmptyFilter {
    pub(crate) fn should_suppress(&mut self, start: usize, end: usize) -> bool {
        let is_empty = start == end;
        if is_empty && self.prev_nonempty_end == Some(start) {
            self.prev_nonempty_end = None;
            return true;
        }
        self.prev_nonempty_end = if is_empty { None } else { Some(end) };
        false
    }
}

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

pub fn expand_replacement(caps: &Captures, replacement: &str, text: &str) -> String {
    fn push_group(result: &mut String, caps: &Captures, name: &str, text: &str) {
        let m = match name.parse::<usize>() {
            Ok(idx) => caps.get(idx),
            Err(_) => caps.get_named(name),
        };
        if let Some(m) = m {
            result.push_str(m.as_str(text));
        }
    }

    let mut result = String::with_capacity(replacement.len());
    let bytes = replacement.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            let start = i;
            while i < bytes.len() && bytes[i] != b'$' {
                i += 1;
            }
            result.push_str(&replacement[start..i]);
            continue;
        }
        if i + 1 >= bytes.len() {
            result.push('$');
            i += 1;
            continue;
        }
        match bytes[i + 1] {
            b'$' => {
                result.push('$');
                i += 2;
            }
            b'{' => {
                if let Some(close) = replacement[i + 2..].find('}') {
                    let name = &replacement[i + 2..i + 2 + close];
                    push_group(&mut result, caps, name, text);
                    i = i + 2 + close + 1;
                } else {
                    // No closing brace - not a valid reference, keep literally.
                    result.push('$');
                    i += 1;
                }
            }
            c if c.is_ascii_digit() => {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                push_group(&mut result, caps, &replacement[start..j], text);
                i = j;
            }
            c if c == b'_' || c.is_ascii_alphabetic() => {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && (bytes[j] == b'_' || bytes[j].is_ascii_alphanumeric()) {
                    j += 1;
                }
                push_group(&mut result, caps, &replacement[start..j], text);
                i = j;
            }
            _ => {
                result.push('$');
                i += 1;
            }
        }
    }
    result
}
