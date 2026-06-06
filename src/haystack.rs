/// A trait for text that can be searched by the regex engine.
pub trait Haystack: Copy + Clone {
    type Cursor: HaystackCursor;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn cursor_at(&self, pos: usize) -> Self::Cursor;

    fn char_at(&self, pos: usize) -> Option<(char, usize)>;

    fn char_before(&self, pos: usize) -> Option<char>;

    fn starts_with(&self, pos: usize, literal: &str) -> bool;

    fn matches_range(&self, pos: usize, other_start: usize, other_end: usize) -> bool;

    fn find_byte(&self, byte: u8, pos: usize) -> Option<usize>;

    /// Return a contiguous byte slice for the entire haystack, if available.
    /// Non-contiguous haystacks (e.g. ropes) should return `None`.
    fn as_bytes_opt(&self) -> Option<&[u8]> {
        None
    }
}

pub trait HaystackCursor: Iterator<Item = char> + Clone {
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
        let bytes = self.as_bytes();
        // Scan backwards over UTF-8 continuation bytes (0b10xxxxxx)
        let mut i = pos - 1;
        while i > 0 && bytes[i] & 0xC0 == 0x80 {
            i -= 1;
        }
        self[i..pos].chars().next()
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

    #[inline]
    fn find_byte(&self, byte: u8, pos: usize) -> Option<usize> {
        if pos >= self.len() {
            return None;
        }
        memchr::memchr(byte, &self.as_bytes()[pos..]).map(|i| i + pos)
    }

    #[inline]
    fn as_bytes_opt(&self) -> Option<&[u8]> {
        Some(str::as_bytes(self))
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
