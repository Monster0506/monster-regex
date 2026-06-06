pub mod compiler;
pub mod nfa;
mod tests;
pub mod vm;

use crate::captures::{Captures, Match};
use crate::engine::{CompiledRegex, CompiledRegexHaystack, RegexEngine};
use crate::errors::CompileError;
use crate::flags::Flags;
use crate::haystack::Haystack;
use crate::parser::{AstNode, CharClass, CharRange, Parser};
use compiler::Compiler;
use nfa::{Nfa, State};
use std::collections::HashMap;
use vm::{Literal, PikeVM, StartFilter};

/// Linear engine using Thompson NFA and Pike VM.
#[derive(Clone, Copy, Debug, Default)]
pub struct LinearRegexEngine;

impl RegexEngine for LinearRegexEngine {
    type Regex = LinearRegex;

    fn compile(&self, pattern: &str, flags: Flags) -> Result<Self::Regex, CompileError> {
        LinearRegex::new(pattern, flags)
    }
}

fn analyze_captures(nodes: &[AstNode]) -> (usize, HashMap<String, usize>) {
    let mut count = 0;
    let mut map = HashMap::new();
    visit_ast(nodes, &mut count, &mut map);
    (count, map)
}

fn visit_ast(nodes: &[AstNode], count: &mut usize, map: &mut HashMap<String, usize>) {
    for node in nodes {
        match node {
            AstNode::Group {
                index,
                nodes,
                capture,
                name,
            } => {
                if *capture && let Some(i) = index {
                    *count = (*count).max(*i);
                    if let Some(n) = name {
                        map.insert(n.clone(), *i);
                    }
                }
                visit_ast(nodes, count, map);
            }
            AstNode::Alternation(alts) => {
                for alt in alts {
                    visit_ast(alt, count, map);
                }
            }
            AstNode::ZeroOrMore { node, .. }
            | AstNode::OneOrMore { node, .. }
            | AstNode::Optional { node, .. }
            | AstNode::Exact { node, .. }
            | AstNode::Range { node, .. } => {
                visit_ast(std::slice::from_ref(node), count, map);
            }
            AstNode::LookAhead { nodes, .. } | AstNode::LookBehind { nodes, .. } => {
                visit_ast(nodes, count, map);
            }
            _ => {}
        }
    }
}

// -- Compiled regex -------------------------------------------------------------

pub struct LinearRegex {
    vm: PikeVM,
    pattern: String,
    flags: Flags,
    // Reserved for capture-group support (not yet read by the linear engine).
    #[allow(dead_code)]
    group_count: usize,
    #[allow(dead_code)]
    named_groups: HashMap<String, usize>,
}

impl LinearRegex {
    /// Concrete, stack-allocated find-all iterator (no heap allocation for literal patterns).
    pub fn find_all_linear<'a>(&'a self, text: &'a str) -> LinearFindAll<'a> {
        if let Some(lit) = self.vm.literal() {
            if !lit.case_insensitive {
                // CS: single FindIter streams through the whole document.
                return LinearFindAll::Literal {
                    inner: lit.finder.find_iter(text.as_bytes()),
                    lit_len: lit.len(),
                };
            }
            // CI: memchr2 on first byte + verify (existing path).
            if let Some(iter) = self.vm.literal_find_all(text.as_bytes(), 0) {
                return LinearFindAll::LiteralCI(iter);
            }
        }
        LinearFindAll::Nfa(FindMatchesIterator {
            text,
            regex: self,
            last_end: 0,
        })
    }

    pub fn new(pattern: &str, mut flags: Flags) -> Result<Self, CompileError> {
        // Smartcase: all-lowercase pattern -> case-insensitive
        if flags.ignore_case.is_none() {
            let has_uppercase = pattern.chars().any(|c| c.is_uppercase());
            flags.ignore_case = Some(!has_uppercase);
        }

        let mut parser = Parser::new(pattern, flags);
        let ast = parser
            .parse()
            .map_err(|e| CompileError::InvalidPattern(e.to_string()))?;

        let (group_count, named_groups) = analyze_captures(&ast);

        let start_filter = analyze_start_filter(&ast, &flags);

        let compiler = Compiler::new(flags);
        let nfa = compiler.compile(&ast)?;

        // Try to extract a pure literal from the compiled NFA (bypasses simulation).
        let literal = extract_literal(&nfa);

        let vm = PikeVM::new(nfa, start_filter, literal);

        Ok(LinearRegex {
            vm,
            pattern: pattern.to_string(),
            flags,
            group_count,
            named_groups,
        })
    }
}

// -- Start-filter analysis ------------------------------------------------------

/// Collect possible start bytes from an AST node (up to 3; returns empty if too many).
fn collect_start_bytes(node: &AstNode, ic: bool) -> Vec<u8> {
    match node {
        AstNode::Literal(c) => {
            if !c.is_ascii() {
                return vec![];
            }
            let b = *c as u8;
            if ic {
                let lo = b.to_ascii_lowercase();
                let up = b.to_ascii_uppercase();
                if lo == up { vec![lo] } else { vec![lo, up] }
            } else {
                vec![b]
            }
        }
        AstNode::CharClass(CharClass::Set {
            chars,
            negated: false,
        }) => extract_bytes_from_ranges(chars, ic),
        AstNode::CharClass(_) => vec![],
        AstNode::OneOrMore { node, .. } | AstNode::Exact { node, .. } => {
            collect_start_bytes(node, ic)
        }
        AstNode::Group { nodes, .. } => {
            if nodes.is_empty() {
                vec![]
            } else {
                collect_start_bytes(&nodes[0], ic)
            }
        }
        _ => vec![],
    }
}

fn extract_bytes_from_ranges(ranges: &[CharRange], ic: bool) -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    for r in ranges {
        let span = r.end as u32 - r.start as u32;
        if span > 4 || !r.start.is_ascii() || !r.end.is_ascii() {
            return vec![]; // range too wide or non-ASCII
        }
        let mut c = r.start as u8;
        loop {
            if ic {
                let lo = c.to_ascii_lowercase();
                let up = c.to_ascii_uppercase();
                bytes.push(lo);
                if lo != up {
                    bytes.push(up);
                }
            } else {
                bytes.push(c);
            }
            if c == r.end as u8 {
                break;
            }
            c += 1;
        }
    }
    bytes
}

fn analyze_start_filter(nodes: &[AstNode], flags: &Flags) -> StartFilter {
    if nodes.is_empty() {
        return StartFilter::None;
    }
    let ic = flags.ignore_case.unwrap_or(false);

    // Check if the leading node is a wide class that maps to a byte range.
    if let Some(range_filter) = start_filter_from_class(&nodes[0]) {
        return range_filter;
    }

    let mut bytes = collect_start_bytes(&nodes[0], ic);
    bytes.sort_unstable();
    bytes.dedup();
    match bytes.len() {
        0 => StartFilter::None,
        1 => StartFilter::One(bytes[0]),
        2 => StartFilter::Two(bytes[0], bytes[1]),
        3 => StartFilter::Three(bytes[0], bytes[1], bytes[2]),
        _ => StartFilter::None, // > 3 possible starts: too many for memchr3
    }
}

/// Map well-known classes to an efficient ByteRange/Table128 filter.
fn start_filter_from_class(node: &AstNode) -> Option<StartFilter> {
    let class = match node {
        AstNode::CharClass(c) => c,
        AstNode::OneOrMore { node, .. } | AstNode::Exact { node, .. } => {
            return start_filter_from_class(node);
        }
        AstNode::Group { nodes, .. } => {
            return nodes.first().and_then(start_filter_from_class);
        }
        _ => return None,
    };
    match class {
        // \d -> '0'..='9'
        CharClass::Digit => Some(StartFilter::ByteRange(b'0', b'9')),
        // \w -> [0-9A-Za-z_] - arbitrary set, use Table128 (case-insensitivity
        // does not change membership of this class)
        CharClass::Word => Some(table128_for(|b: u8| {
            let c = b as char;
            c.is_ascii_alphanumeric() || c == '_'
        })),
        // \p{Alpha} / [a-zA-Z]
        CharClass::Lowercase => Some(StartFilter::ByteRange(b'a', b'z')),
        CharClass::Uppercase => Some(StartFilter::ByteRange(b'A', b'Z')),
        CharClass::Alphanumeric => Some(table128_for(|b: u8| (b as char).is_ascii_alphanumeric())),
        // Whitespace: common ASCII whitespace bytes
        CharClass::Whitespace => Some(table128_for(|b: u8| {
            matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0C | 0x0B)
        })),
        _ => None,
    }
}

fn table128_for(pred: impl Fn(u8) -> bool) -> StartFilter {
    let mut mask = [0u64; 2];
    for b in 0u8..=127u8 {
        if pred(b) {
            mask[(b >> 6) as usize] |= 1u64 << (b & 63);
        }
    }
    StartFilter::Table128(mask)
}

// -- Literal extraction from compiled NFA --------------------------------------

/// Walk the NFA from its start state. If the entire pattern is a chain of
/// Char/Class-single-char states ending in Match, extract as a Literal.
fn extract_literal(nfa: &Nfa) -> Option<Literal> {
    let mut pos = nfa.start;
    let mut bytes: Vec<u8> = Vec::new();
    let mut is_ci = false;

    // Guard against pathological NFA shapes (e.g. empty pattern)
    let max_steps = nfa.states.len() + 1;
    for _ in 0..max_steps {
        match &nfa.states[pos] {
            State::Char(c, next) if c.is_ascii() => {
                bytes.push(*c as u8);
                pos = *next;
            }
            State::Class(
                CharClass::Set {
                    chars,
                    negated: false,
                },
                next,
            ) => {
                // Accept exactly a single-char or {lower, upper} set
                if let Some((b, ci)) = single_char_from_set(chars) {
                    bytes.push(b);
                    is_ci |= ci;
                    pos = *next;
                } else {
                    return None;
                }
            }
            State::Match => {
                return if bytes.is_empty() {
                    None
                } else {
                    Some(Literal::new(bytes.into_boxed_slice(), is_ci))
                };
            }
            _ => return None,
        }
    }
    None
}

/// If a `Set` represents exactly one ASCII character (case-sensitive or case pair),
/// return `(lowercase_byte, is_case_insensitive)`.
fn single_char_from_set(chars: &[CharRange]) -> Option<(u8, bool)> {
    match chars.len() {
        1 => {
            let r = &chars[0];
            if r.start == r.end && r.start.is_ascii() {
                Some((r.start as u8, false))
            } else {
                None
            }
        }
        2 => {
            let (r1, r2) = (&chars[0], &chars[1]);
            if r1.start == r1.end && r2.start == r2.end {
                let b1 = r1.start;
                let b2 = r2.start;
                if b1.is_ascii() && b2.is_ascii() {
                    let l1 = (b1 as u8).to_ascii_lowercase();
                    let l2 = (b2 as u8).to_ascii_lowercase();
                    if l1 == l2 {
                        // Case pair (e.g. 'f' and 'F')
                        return Some((l1, true));
                    }
                }
            }
            None
        }
        _ => None,
    }
}

// -- CompiledRegex impl ---------------------------------------------------------

impl CompiledRegex for LinearRegex {
    fn pattern(&self) -> &str {
        &self.pattern
    }

    fn flags(&self) -> &Flags {
        &self.flags
    }

    fn is_match(&self, text: &str) -> bool {
        self.is_match_from(text)
    }

    fn find(&self, text: &str) -> Option<Match> {
        self.find_from(text)
    }

    fn find_all<'a>(&'a self, text: &'a str) -> Box<dyn Iterator<Item = Match> + 'a> {
        // Fast path: pure literal - single-pass memmem scan (avoids 80K NFA restarts).
        if let Some(iter) = self.vm.literal_find_all(text.as_bytes(), 0) {
            return Box::new(iter);
        }
        Box::new(FindMatchesIterator {
            text,
            regex: self,
            last_end: 0,
        })
    }

    fn captures(&self, text: &str) -> Option<Captures> {
        let full = self.vm.find_raw(text, 0)?;
        Some(Captures {
            full_match: full,
            groups: vec![],
            named: HashMap::new(),
        })
    }

    fn captures_all<'a>(&'a self, text: &'a str) -> Box<dyn Iterator<Item = Captures> + 'a> {
        Box::new(CapturesIterator {
            text,
            regex: self,
            last_end: 0,
        })
    }

    fn replace(&self, text: &str, replacement: &str) -> String {
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

    fn replace_all(&self, text: &str, replacement: &str) -> String {
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
}

impl crate::engine::CompiledRegexHaystack for LinearRegex {
    fn is_match_from<H: Haystack>(&self, haystack: H) -> bool {
        self.vm.find_from(haystack, 0).is_some()
    }

    fn find_from<H: Haystack>(&self, haystack: H) -> Option<Match> {
        self.vm.find_from(haystack, 0)
    }

    fn find_from_at<H: Haystack>(&self, haystack: H, start: usize) -> Option<Match> {
        self.vm.find_from(haystack, start)
    }

    fn find_all_from<'a, H: Haystack + 'a>(
        &'a self,
        haystack: H,
    ) -> Box<dyn Iterator<Item = Match> + 'a> {
        Box::new(FindMatchesIterator {
            text: haystack,
            regex: self,
            last_end: 0,
        })
    }
}

// -- Concrete find-all iterator (no Box, no vtable) ----------------------------

/// Stack-allocated iterator returned by `Regex<LinearRegexEngine>::find_all`.
///
/// `Literal`: one `memmem::FindIter` streams the whole haystack - no heap
/// allocation, no vtable dispatch, one enum-dispatch branch per match.
/// `Nfa`: falls back to per-match `find_from_at`.
pub enum LinearFindAll<'a> {
    Literal {
        inner: memchr::memmem::FindIter<'a, 'a>,
        lit_len: usize,
    },
    LiteralCI(vm::LiteralFindIter<'a, 'a>),
    Nfa(FindMatchesIterator<'a, &'a str>),
}

impl<'a> Iterator for LinearFindAll<'a> {
    type Item = Match;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Literal { inner, lit_len } => {
                let start = inner.next()?;
                Some(Match {
                    start,
                    end: start + *lit_len,
                })
            }
            Self::LiteralCI(it) => it.next(),
            Self::Nfa(it) => it.next(),
        }
    }
}

// -- Iterators ------------------------------------------------------------------

pub struct FindMatchesIterator<'a, H: Haystack> {
    pub(crate) text: H,
    pub(crate) regex: &'a LinearRegex,
    pub(crate) last_end: usize,
}

impl<'a, H: Haystack> Iterator for FindMatchesIterator<'a, H> {
    type Item = Match;

    fn next(&mut self) -> Option<Self::Item> {
        if self.last_end > self.text.len() {
            return None;
        }
        let m = self.regex.find_from_at(self.text, self.last_end)?;
        self.last_end = m.end.max(m.start + 1);
        Some(m)
    }
}

struct CapturesIterator<'a> {
    text: &'a str,
    regex: &'a LinearRegex,
    last_end: usize,
}

impl<'a> Iterator for CapturesIterator<'a> {
    type Item = Captures;

    fn next(&mut self) -> Option<Self::Item> {
        if self.last_end > self.text.len() {
            return None;
        }
        let slice = &self.text[self.last_end..];
        let mut caps = self.regex.captures(slice)?;
        let offset = self.last_end;
        caps.full_match.start += offset;
        caps.full_match.end += offset;
        for g in caps.groups.iter_mut().flatten() {
            g.start += offset;
            g.end += offset;
        }
        for g in caps.named.values_mut() {
            g.start += offset;
            g.end += offset;
        }
        self.last_end = caps.full_match.end.max(caps.full_match.start + 1);
        Some(caps)
    }
}
