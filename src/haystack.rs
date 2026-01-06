/// A trait for text that can be searched by the regex engine.
/// This abstraction allows searching over non-contiguous memory (like ropes)
/// without flattening to a single string.
pub trait Haystack: Copy + Clone {
    /// Total length of the haystack in bytes
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the char at the specified byte position
    /// Returns (char, len_in_bytes)
    fn char_at(&self, pos: usize) -> Option<(char, usize)>;

    /// Get the char/byte length *before* the specified position.
    /// Useful for lookbehind and boundary checks.
    fn char_before(&self, pos: usize) -> Option<char>;

    /// Check if the haystack starts with the literal string at the given position
    fn starts_with(&self, pos: usize, literal: &str) -> bool;

    /// Check if the range at `pos` matches the content of the range `other_start..other_end`
    /// Used for backreferences.
    fn matches_range(&self, pos: usize, other_start: usize, other_end: usize) -> bool;
}

impl<'a> Haystack for &'a str {
    #[inline]
    fn len(&self) -> usize {
        str::len(self)
    }

    #[inline]
    fn char_at(&self, pos: usize) -> Option<(char, usize)> {
        if pos >= self.len() {
            return None;
        }
        let c = self[pos..].chars().next()?;
        Some((c, c.len_utf8()))
    }

    #[inline]
    fn char_before(&self, pos: usize) -> Option<char> {
        if pos == 0 || pos > self.len() {
            return None;
        }
        self[..pos].chars().last()
    }

    #[inline]
    fn starts_with(&self, pos: usize, literal: &str) -> bool {
        if pos > self.len() {
            return false;
        }
        self[pos..].starts_with(literal)
    }

    #[inline]
    fn matches_range(&self, pos: usize, other_start: usize, other_end: usize) -> bool {
        if other_end > self.len() || other_start > other_end {
            return false;
        }
        let substring = &self[other_start..other_end];
        self.starts_with(pos, substring)
    }
}
