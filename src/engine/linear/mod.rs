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
        if let Some(fixed_len) = self.vm.fixed_len()
            && let Some(iter) = self.vm.find_all_fixed_len(text.as_bytes(), 0, fixed_len)
        {
            return LinearFindAll::FixedLen(iter);
        }
        if let Some(iter) = self.vm.find_all_segments(text.as_bytes(), 0) {
            return LinearFindAll::Segments(iter);
        }
        if let Some(iter) = self.vm.find_all_bitparallel(text.as_bytes(), 0) {
            return LinearFindAll::BitParallel(iter);
        }
        LinearFindAll::Nfa(FindMatchesIterator {
            text,
            regex: self,
            last_end: 0,
        })
    }

    pub fn new(pattern: &str, mut flags: Flags) -> Result<Self, CompileError> {
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
        let fixed_len = fixed_length(&ast);
        let segments = disjoint_greedy_segments(&ast, &flags);

        let compiler = Compiler::new(flags);
        let nfa = compiler.compile(&ast)?;

        let literal = extract_literal(&nfa);

        let vm = PikeVM::new(nfa, start_filter, literal, fixed_len, segments);

        Ok(LinearRegex {
            vm,
            pattern: pattern.to_string(),
            flags,
            group_count,
            named_groups,
        })
    }
}

fn fixed_length(nodes: &[AstNode]) -> Option<usize> {
    nodes.iter().try_fold(0usize, |acc, n| {
        Some(acc + fixed_length_node(n)?)
    })
}

fn fixed_length_node(node: &AstNode) -> Option<usize> {
    match node {
        AstNode::Literal(_) | AstNode::CharClass(_) => Some(1),
        AstNode::Exact { node, count } => Some(fixed_length_node(node)? * count),
        AstNode::Range {
            node,
            min,
            max: Some(max),
            ..
        } if min == max => Some(fixed_length_node(node)? * min),
        AstNode::Group { nodes, .. } => fixed_length(nodes),
        AstNode::Alternation(alts) => {
            let mut lens = alts.iter().map(|alt| fixed_length(alt));
            let first = lens.next()??;
            lens.all(|l| l == Some(first)).then_some(first)
        }
        AstNode::ZeroOrMore { .. }
        | AstNode::OneOrMore { .. }
        | AstNode::Optional { .. }
        | AstNode::Range { .. }
        | AstNode::StartAnchor
        | AstNode::EndAnchor
        | AstNode::WordBoundary
        | AstNode::StartWord
        | AstNode::EndWord
        | AstNode::SetMatchStart
        | AstNode::SetMatchEnd
        | AstNode::Backref(_)
        | AstNode::LookAhead { .. }
        | AstNode::LookBehind { .. } => None,
    }
}

/// A 256-bit set of bytes, used as a quantified atom's alphabet.
#[derive(Clone, Copy, Debug)]
pub struct SegBits([u64; 4]);

impl SegBits {
    fn empty() -> Self {
        SegBits([0; 4])
    }
    fn set(&mut self, b: u8) {
        self.0[(b / 64) as usize] |= 1u64 << (b % 64);
    }
    #[inline]
    pub fn contains(&self, b: u8) -> bool {
        (self.0[(b / 64) as usize] >> (b % 64)) & 1 != 0
    }
    fn disjoint(&self, other: &SegBits) -> bool {
        self.0.iter().zip(other.0.iter()).all(|(a, b)| a & b == 0)
    }
    fn union(&self, other: &SegBits) -> SegBits {
        let mut r = [0u64; 4];
        for i in 0..4 {
            r[i] = self.0[i] | other.0[i];
        }
        SegBits(r)
    }
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub alphabet: SegBits,
    pub min: usize,
    pub max: usize,
}

fn atom_alphabet(node: &AstNode, ic: bool, dotall: bool) -> Option<SegBits> {
    match node {
        AstNode::Literal(c) => {
            if !c.is_ascii() {
                return None;
            }
            let mut s = SegBits::empty();
            let b = *c as u8;
            if ic {
                s.set(b.to_ascii_lowercase());
                s.set(b.to_ascii_uppercase());
            } else {
                s.set(b);
            }
            Some(s)
        }
        AstNode::CharClass(cls) => {
            let mut s = SegBits::empty();
            for byte in 0u8..=127u8 {
                if vm::matches_class_static(cls, byte as char, ic, dotall) {
                    s.set(byte);
                }
            }
            Some(s)
        }
        _ => None,
    }
}

fn disjoint_greedy_segments(nodes: &[AstNode], flags: &Flags) -> Option<Vec<Segment>> {
    if nodes.is_empty() {
        return None;
    }
    let ic = flags.ignore_case.unwrap_or(false);
    let dotall = flags.dotall;
    let mut segs = Vec::with_capacity(nodes.len());
    for node in nodes {
        let (inner, min, max, greedy): (&AstNode, usize, usize, bool) = match node {
            AstNode::Literal(_) | AstNode::CharClass(_) => (node, 1, 1, true),
            AstNode::OneOrMore { node, greedy } => (node.as_ref(), 1, usize::MAX, *greedy),
            AstNode::ZeroOrMore { node, greedy } => (node.as_ref(), 0, usize::MAX, *greedy),
            AstNode::Optional { node, greedy } => (node.as_ref(), 0, 1, *greedy),
            AstNode::Exact { node, count } => (node.as_ref(), *count, *count, true),
            AstNode::Range {
                node,
                min,
                max,
                greedy,
            } => (node.as_ref(), *min, max.unwrap_or(usize::MAX), *greedy),
            _ => return None,
        };
        if !greedy {
            return None;
        }
        let alphabet = atom_alphabet(inner, ic, dotall)?;
        segs.push(Segment { alphabet, min, max });
    }

    let mut reachable = SegBits::empty();
    let mut reachable_active = false;
    for seg in &segs {
        if reachable_active && !seg.alphabet.disjoint(&reachable) {
            return None;
        }
        reachable = if seg.min == 0 && reachable_active {
            reachable.union(&seg.alphabet)
        } else {
            seg.alphabet
        };
        reachable_active = true;
    }

    Some(segs)
}

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

/// Map a leading character class to an efficient byte start-filter.
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
        CharClass::Digit => Some(StartFilter::ByteRange(b'0', b'9')),
        _ => None,
    }
}

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
        if let Some(iter) = self.vm.literal_find_all(text.as_bytes(), 0) {
            return Box::new(iter);
        }
        if let Some(fixed_len) = self.vm.fixed_len()
            && let Some(iter) = self.vm.find_all_fixed_len(text.as_bytes(), 0, fixed_len)
        {
            return Box::new(iter);
        }
        if let Some(iter) = self.vm.find_all_segments(text.as_bytes(), 0) {
            return Box::new(iter);
        }
        if let Some(iter) = self.vm.find_all_bitparallel(text.as_bytes(), 0) {
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

pub enum LinearFindAll<'a> {
    Literal {
        inner: memchr::memmem::FindIter<'a, 'a>,
        lit_len: usize,
    },
    LiteralCI(vm::LiteralFindIter<'a, 'a>),
    FixedLen(vm::FixedLenFindAll<'a, 'a>),
    Segments(vm::SegmentsFindAll<'a, 'a>),
    BitParallel(vm::BitParallelFindAll<'a, 'a>),
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
            Self::FixedLen(it) => it.next(),
            Self::Segments(it) => it.next(),
            Self::BitParallel(it) => it.next(),
            Self::Nfa(it) => it.next(),
        }
    }
}

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
