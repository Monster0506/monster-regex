#[cfg(test)]
mod tests {
    use super::super::compiler::Compiler;
    use super::super::vm::PikeVM;
    use crate::flags::Flags;
    use crate::parser::Parser;

    fn compile_and_run(pattern: &str, text: &str) -> Option<(usize, usize)> {
        let flags = Flags::default();
        let mut parser = Parser::new(pattern, flags.clone());
        let ast = parser.parse().expect("Parse failed");

        let compiler = Compiler::new(flags);
        let nfa = compiler.compile(&ast).expect("Compile failed");

        let vm = PikeVM::new(nfa);
        let m = vm.find_from(text, 0)?;
        Some((m.start, m.end))
    }

    fn compile_and_run_flags(pattern: &str, text: &str, flags: Flags) -> Option<(usize, usize)> {
        let mut parser = Parser::new(pattern, flags.clone());
        let ast = parser.parse().expect("Parse failed");

        let compiler = Compiler::new(flags);
        let nfa = compiler.compile(&ast).expect("Compile failed");

        let vm = PikeVM::new(nfa);
        let m = vm.find_from(text, 0)?;
        Some((m.start, m.end))
    }

    #[test]
    fn test_literal() {
        assert_eq!(compile_and_run("abc", "abc"), Some((0, 3)));
        assert_eq!(compile_and_run("abc", "xabcy"), Some((1, 4)));
        assert_eq!(compile_and_run("abc", "ab"), None);
    }

    #[test]
    fn test_alternation() {
        assert_eq!(compile_and_run("cat|dog", "dog"), Some((0, 3)));
        assert_eq!(compile_and_run("cat|dog", "cat"), Some((0, 3)));
        assert_eq!(compile_and_run("a|b|c", "b"), Some((0, 1)));
    }

    #[test]
    fn test_quantifiers() {
        assert_eq!(compile_and_run("a*", "aaa"), Some((0, 3)));
        assert_eq!(compile_and_run("a*", "b"), Some((0, 0))); // Matches empty at start
        assert_eq!(compile_and_run("a+", "aaa"), Some((0, 3)));
        assert_eq!(compile_and_run("a+", "b"), None); // find_from(0) fails if no match found anywhere? 
        // My VM implementation currently adds start state at every position.
        // So it should find NO match if 'a+' is not in 'b'.
        // Wait, 'a*' in 'b' -> matches empty string at 0.

        // ?
        assert_eq!(compile_and_run("a?", "a"), Some((0, 1)));
        assert_eq!(compile_and_run("a?", "b"), Some((0, 0)));

        assert_eq!(compile_and_run("(ab)+", "abab"), Some((0, 4)));

        // Bounded
        assert_eq!(compile_and_run("a{2}", "aa"), Some((0, 2)));
        assert_eq!(compile_and_run("a{2}", "a"), None);
        assert_eq!(compile_and_run("a{2,3}", "aaa"), Some((0, 3)));
        assert_eq!(compile_and_run("a{2,3}", "aa"), Some((0, 2)));
        assert_eq!(compile_and_run("a{2,}", "aaaa"), Some((0, 4)));
    }

    #[test]
    fn test_dot() {
        assert_eq!(compile_and_run(".", "a"), Some((0, 1)));
        assert_eq!(compile_and_run(".", "\n"), None);
    }

    #[test]
    fn test_anchors() {
        assert_eq!(compile_and_run("^a", "a"), Some((0, 1)));
        assert_eq!(compile_and_run("^a", "ba"), None);
        assert_eq!(compile_and_run("a$", "a"), Some((0, 1)));
        assert_eq!(compile_and_run("a$", "ab"), None);
        // Both
        assert_eq!(compile_and_run("^abc$", "abc"), Some((0, 3)));
    }

    // Mock Haystack to verify compatibility with non-contiguous memory
    #[derive(Copy, Clone)]
    struct MockHaystack<'a> {
        first: &'a str,
        second: &'a str,
    }

    impl<'a> MockHaystack<'a> {
        fn new(first: &'a str, second: &'a str) -> Self {
            Self { first, second }
        }
    }

    #[derive(Clone)]
    struct MockCursor<'a> {
        haystack: MockHaystack<'a>,
        pos: usize,
    }

    impl<'a> Iterator for MockCursor<'a> {
        type Item = char;
        fn next(&mut self) -> Option<Self::Item> {
            let (c, len) = self.haystack.char_at(self.pos)?;
            self.pos += len;
            Some(c)
        }
    }

    impl<'a> crate::haystack::HaystackCursor for MockCursor<'a> {
        fn peek(&self) -> Option<char> {
            self.haystack.char_at(self.pos).map(|(c, _)| c)
        }
    }

    impl<'a> crate::haystack::Haystack for MockHaystack<'a> {
        type Cursor = MockCursor<'a>;

        fn len(&self) -> usize {
            self.first.len() + self.second.len()
        }

        fn cursor_at(&self, pos: usize) -> Self::Cursor {
            MockCursor {
                haystack: *self,
                pos,
            }
        }

        fn char_at(&self, pos: usize) -> Option<(char, usize)> {
            if pos < self.first.len() {
                // In first chunk
                let slice = &self.first[pos..];
                let c = slice.chars().next()?;
                Some((c, c.len_utf8()))
            } else {
                // In second chunk
                let offset = pos - self.first.len();
                if offset >= self.second.len() {
                    return None;
                }
                let slice = &self.second[offset..];
                let c = slice.chars().next()?;
                Some((c, c.len_utf8()))
            }
        }

        fn char_before(&self, pos: usize) -> Option<char> {
            if pos == 0 {
                return None;
            }
            if pos <= self.first.len() {
                self.first[..pos].chars().last()
            } else {
                let offset = pos - self.first.len();
                if offset == 0 {
                    // Last char of first
                    self.first.chars().last()
                } else {
                    self.second[..offset].chars().last()
                }
            }
        }

        fn starts_with(&self, _pos: usize, _literal: &str) -> bool {
            unimplemented!("Not needed for linear engine basic tests")
        }

        fn matches_range(&self, _pos: usize, _other_start: usize, _other_end: usize) -> bool {
            unimplemented!("Not needed for linear engine basic tests")
        }
    }

    #[test]
    fn test_haystack_compatibility() {
        let haystack = MockHaystack::new("abc", "def");
        let pattern = "bcd";

        let flags = Flags::default();
        let mut parser = Parser::new(pattern, flags.clone());
        let ast = parser.parse().expect("Parse failed");

        let compiler = Compiler::new(flags);
        let nfa = compiler.compile(&ast).expect("Compile failed");

        let vm = PikeVM::new(nfa);
        let m = vm.find_from(haystack, 0).expect("Match not found");

        // b at index 1, c at 2, d at 3 (start of second chunk)
        assert_eq!(m.start, 1);
        assert_eq!(m.end, 4);
    }

    #[test]
    fn test_word_boundaries() {
        assert_eq!(compile_and_run(r"\bword\b", " word "), Some((1, 5)));
        assert_eq!(compile_and_run(r"\bword\b", "sword"), None);
        assert_eq!(compile_and_run(r"\<word", "word"), Some((0, 4)));
        assert_eq!(compile_and_run(r"word\>", "word"), Some((0, 4)));
    }

    #[test]
    fn test_classes() {
        assert_eq!(compile_and_run("[a-c]", "b"), Some((0, 1)));
        assert_eq!(compile_and_run("[a-c]", "d"), None);

        assert_eq!(compile_and_run(r"\d", "1"), Some((0, 1)));
        assert_eq!(compile_and_run(r"\d", "a"), None);

        assert_eq!(compile_and_run(r"\w", "_"), Some((0, 1)));
    }

    #[test]
    fn test_flags() {
        let mut flags = Flags::default();
        flags.ignore_case = Some(true);
        assert_eq!(compile_and_run_flags("a", "A", flags.clone()), Some((0, 1)));

        let mut flags = Flags::default();
        flags.dotall = true;
        assert_eq!(compile_and_run_flags(".", "\n", flags), Some((0, 1)));

        let mut flags = Flags::default();
        flags.ignore_case = Some(true);
        assert_eq!(compile_and_run_flags("[a-z]", "A", flags), Some((0, 1)));
    }

    #[test]
    fn test_leftmost_longest() {
        assert_eq!(compile_and_run("a*", "aaa"), Some((0, 3)));

        assert_eq!(compile_and_run("a|ab", "ab"), Some((0, 2)));
    }
}
