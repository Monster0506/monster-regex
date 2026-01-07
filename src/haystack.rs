/// A trait for text that can be searched by the regex engine.
/// This abstraction allows searching over non-contiguous memory (like ropes)
/// without flattening to a single string.
pub trait Haystack: Copy + Clone {
    type Cursor: HaystackCursor;

    /// Total length of the haystack in bytes
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get a cursor for streaming access starting at `pos`
    fn cursor_at(&self, pos: usize) -> Self::Cursor;

    /// Get character at position
    fn char_at(&self, pos: usize) -> Option<(char, usize)>;

    /// Get character before position
    fn char_before(&self, pos: usize) -> Option<char>;

    /// Check if haystack starts with literal at pos
    fn starts_with(&self, pos: usize, literal: &str) -> bool;

    /// Check if range matches another range
    fn matches_range(&self, pos: usize, other_start: usize, other_end: usize) -> bool;
}

pub trait HaystackCursor: Iterator<Item = char> + Clone {
    /// Peek at the next character without advancing
    fn peek(&self) -> Option<char>;
}

impl<'a> Haystack for &'a str {
    type Cursor = StrCursor<'a>;

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

    #[inline]
    fn cursor_at(&self, pos: usize) -> Self::Cursor {
        StrCursor {
            chars: self[pos..].chars(),
        }
    }
}

#[derive(Clone)]
pub struct StrCursor<'a> {
    chars: std::str::Chars<'a>,
}

impl<'a> Iterator for StrCursor<'a> {
    type Item = char;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.chars.next()
    }
}

impl<'a> HaystackCursor for StrCursor<'a> {
    #[inline]
    fn peek(&self) -> Option<char> {
        self.chars.clone().next()
    }
}
