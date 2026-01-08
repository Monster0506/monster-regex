use crate::haystack::{Haystack, HaystackCursor};
use crate::regex::Regex;

#[derive(Clone, Copy, Debug)]
struct ChunkedHaystack<'a> {
    chunks: &'a [&'a str],
}

impl<'a> ChunkedHaystack<'a> {
    fn new(chunks: &'a [&'a str]) -> Self {
        Self { chunks }
    }
}

#[derive(Clone, Debug)]
struct ChunkedCursor<'a> {
    chunks: &'a [&'a str],
    current_chunk: usize,
    chars: std::str::Chars<'a>,
}

impl<'a> Iterator for ChunkedCursor<'a> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(c) = self.chars.next() {
                return Some(c);
            }
            self.current_chunk += 1;
            if self.current_chunk >= self.chunks.len() {
                return None;
            }
            self.chars = self.chunks[self.current_chunk].chars();
        }
    }
}

impl<'a> HaystackCursor for ChunkedCursor<'a> {
    fn peek(&self) -> Option<char> {
        let mut clone = self.clone();
        clone.next()
    }
}

impl<'a> Haystack for ChunkedHaystack<'a> {
    type Cursor = ChunkedCursor<'a>;

    fn cursor_at(&self, mut pos: usize) -> Self::Cursor {
        let mut current_offset = 0;
        for (i, chunk) in self.chunks.iter().enumerate() {
            let chunk_len = chunk.len();
            if pos < current_offset + chunk_len {
                let local = pos - current_offset;
                return ChunkedCursor {
                    chunks: self.chunks,
                    current_chunk: i,
                    chars: chunk[local..].chars(),
                };
            }
            current_offset += chunk_len;
        }
        // If pos is out of bounds (or at very end)
        ChunkedCursor {
            chunks: self.chunks,
            current_chunk: self.chunks.len(),
            chars: "".chars(),
        }
    }

    fn len(&self) -> usize {
        self.chunks.iter().map(|s| s.len()).sum()
    }

    fn char_at(&self, pos: usize) -> Option<(char, usize)> {
        let mut current_offset = 0;
        for chunk in self.chunks {
            let chunk_len = chunk.len();
            if pos < current_offset + chunk_len {
                let local_pos = pos - current_offset;
                let c = chunk[local_pos..].chars().next()?;
                return Some((c, c.len_utf8()));
            }
            current_offset += chunk_len;
        }
        None
    }

    fn char_before(&self, pos: usize) -> Option<char> {
        if pos == 0 {
            return None;
        }
        // Naive inefficient implementation, but sufficient for testing correctness
        let mut p = 0;
        let mut last = None;
        while p < pos {
            if let Some((c, len)) = self.char_at(p) {
                last = Some(c);
                p += len;
            } else {
                break;
            }
        }
        if p == pos { last } else { None }
    }

    fn starts_with(&self, pos: usize, literal: &str) -> bool {
        let mut p = pos;
        for c in literal.chars() {
            match self.char_at(p) {
                Some((hc, len)) => {
                    if hc != c {
                        return false;
                    }
                    p += len;
                }
                None => return false,
            }
        }
        true
    }

    fn matches_range(&self, pos: usize, other_start: usize, other_end: usize) -> bool {
        let len = other_end - other_start;
        if len == 0 {
            return true;
        }

        let mut p1 = pos;
        let mut p2 = other_start;
        let end_p2 = other_end;

        while p2 < end_p2 {
            let c1 = self.char_at(p1);
            let c2 = self.char_at(p2);
            match (c1, c2) {
                (Some((ch1, l1)), Some((ch2, l2))) => {
                    if ch1 != ch2 {
                        return false;
                    }
                    p1 += l1;
                    p2 += l2;
                }
                _ => return false,
            }
        }
        true
    }
}

#[test]
fn test_chunked_simple_match() {
    let chunks = &["Hel", "lo", " ", "Wor", "ld"];
    let haystack = ChunkedHaystack::new(chunks);
    let regex = Regex::new("World", Default::default()).unwrap();

    let m = regex.find_from(haystack).expect("Should match");
    assert_eq!(m.start, 6);
    assert_eq!(m.end, 11);
}

#[test]
fn test_chunked_boundary_match() {
    let chunks = &["abc", "def"];
    let haystack = ChunkedHaystack::new(chunks);
    let regex = Regex::new("cde", Default::default()).unwrap();

    let m = regex
        .find_from(haystack)
        .expect("Should match across chunks");
    assert_eq!(m.start, 2);
    assert_eq!(m.end, 5);
}

#[test]
fn test_chunked_anchors() {
    let chunks = &["start", "\n", "end"];
    let haystack = ChunkedHaystack::new(chunks);

    let regex = Regex::new("^start", Default::default()).unwrap();
    assert!(regex.find_from(haystack).is_some());

    let regex = Regex::new("end$", Default::default()).unwrap();
    assert!(regex.find_from(haystack).is_some());
}

#[test]
fn test_chunked_word_boundary() {
    let chunks = &["word ", "boundary"];
    let haystack = ChunkedHaystack::new(chunks);

    let regex = Regex::new(r"\bboundary\b", Default::default()).unwrap();
    let m = regex.find_from(haystack).expect("Should match boundary");
    assert_eq!(m.start, 5);
}

#[test]
fn test_chunked_backref() {
    let chunks = &["foo", "...", "foo"];
    let haystack = ChunkedHaystack::new(chunks);

    let regex = Regex::new(r"(foo)...\1", Default::default()).unwrap();
    let m = regex.find_from(haystack).expect("Should match backref");
    assert_eq!(m.start, 0);
    assert_eq!(m.end, 9);
}

#[test]
fn test_find_all_chunks() {
    let chunks = &["aa", "a", "aa"];
    let haystack = ChunkedHaystack::new(chunks);
    let regex = Regex::new("a", Default::default()).unwrap();

    let matches: Vec<_> = regex.find_all_from(haystack).collect();
    assert_eq!(matches.len(), 5);
}
