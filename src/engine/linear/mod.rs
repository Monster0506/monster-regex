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
    group_count: usize,
    named_groups: HashMap<String, usize>,
}

impl LinearRegex {
    /// Concrete, stack-allocated find-all iterator (no heap allocation for literal patterns).
    pub fn find_all_linear<'a>(&'a self, text: &'a str) -> LinearFindAll<'a> {
        let inner = self.find_all_linear_unwrapped(text);
        let inner = if self.vm.has_boundary() {
            LinearFindAllInner::Boundary(BoundaryFilter {
                inner: Box::new(inner),
                text,
                leading: self.vm.leading_boundary(),
                trailing: self.vm.trailing_boundary(),
            })
        } else {
            inner
        };
        LinearFindAll {
            inner,
            adjacent_empty: Default::default(),
        }
    }

    fn find_all_linear_unwrapped<'a>(&'a self, text: &'a str) -> LinearFindAllInner<'a> {
        if let Some(iter) = self.vm.multi_literal_find_all(text.as_bytes(), 0) {
            return LinearFindAllInner::MultiLiteral(iter);
        }
        if let Some(lit) = self.vm.literal() {
            if !lit.case_insensitive {
                // CS: single FindIter streams through the whole document.
                return LinearFindAllInner::Literal {
                    inner: lit.finder.find_iter(text.as_bytes()),
                    lit_len: lit.len(),
                };
            }
            // CI: memchr2 on first byte + verify (existing path).
            if let Some(iter) = self.vm.literal_find_all(text.as_bytes(), 0) {
                return LinearFindAllInner::LiteralCI(iter);
            }
        }
        if let Some(fixed_len) = self.vm.fixed_len()
            && let Some(iter) = self.vm.find_all_fixed_len(text.as_bytes(), 0, fixed_len)
        {
            return LinearFindAllInner::FixedLen(iter);
        }
        if let Some(iter) = self.vm.find_all_segments(text.as_bytes(), 0) {
            return LinearFindAllInner::Segments(iter);
        }
        if let Some(iter) = self.vm.find_all_bitparallel(text.as_bytes(), 0) {
            return LinearFindAllInner::BitParallel(Box::new(iter));
        }
        if let Some(iter) = self.vm.unicode_class_run_find_all(text.as_bytes(), 0) {
            return LinearFindAllInner::UnicodeClassRun(iter);
        }
        LinearFindAllInner::Nfa(FindMatchesIterator {
            text,
            regex: self,
            last_end: 0,
            adjacent_empty: Default::default(),
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

        let has_captures = group_count > 0;

        if !has_captures && let Some((leading, core, trailing)) = split_boundary_wrapper(&ast) {
            let core_start_filter = analyze_start_filter(core, &flags);
            let core_fixed_len = fixed_length(core);
            let core_segments = disjoint_greedy_segments(core, &flags);
            let core_compiler = Compiler::new(flags);
            let core_nfa = core_compiler.compile(core)?;
            let core_literal = extract_literal(&core_nfa);
            let core_multi_literal = alternation_of_literals(core);
            if core_literal.is_some()
                || core_fixed_len.is_some()
                || core_segments.is_some()
                || core_multi_literal.is_some()
            {
                let mut vm = PikeVM::new(
                    core_nfa,
                    core_start_filter,
                    core_literal,
                    core_fixed_len,
                    core_segments,
                    priority_safe(core),
                )
                .with_boundary(leading, trailing);
                if let Some(literals) = core_multi_literal {
                    let ic = flags.ignore_case.unwrap_or(false);
                    vm = vm.with_multi_literal(vm::MultiLiteral::new(literals, ic));
                }
                return Ok(LinearRegex {
                    vm,
                    pattern: pattern.to_string(),
                    flags,
                    group_count,
                    named_groups,
                });
            }
        }

        let start_filter = analyze_start_filter(&ast, &flags);
        let fixed_len = if has_captures {
            None
        } else {
            fixed_length(&ast)
        };
        let segments = if has_captures {
            None
        } else {
            disjoint_greedy_segments(&ast, &flags)
        };

        let compiler = Compiler::new(flags);
        let nfa = compiler.compile(&ast)?;

        let literal = if has_captures {
            None
        } else {
            extract_literal(&nfa)
        };

        let mut vm = PikeVM::new(
            nfa,
            start_filter,
            literal,
            fixed_len,
            segments,
            priority_safe(&ast),
        );
        if !has_captures {
            if let Some(literals) = alternation_of_literals(&ast) {
                let ic = flags.ignore_case.unwrap_or(false);
                vm = vm.with_multi_literal(vm::MultiLiteral::new(literals, ic));
            }
            if let Some((class, min, max)) = unicode_class_run(&ast) {
                let ic = flags.ignore_case.unwrap_or(false);
                vm = vm.with_unicode_class_run(vm::UnicodeClassRun::new(
                    class,
                    ic,
                    flags.dotall,
                    min,
                    max,
                ));
            }
        }

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
    nodes
        .iter()
        .try_fold(0usize, |acc, n| Some(acc + fixed_length_node(n)?))
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
        | AstNode::LookBehind { .. }
        | AstNode::Subroutine(_) => None,
    }
}

fn boundary_kind(node: &AstNode) -> Option<vm::BoundaryKind> {
    match node {
        AstNode::WordBoundary => Some(vm::BoundaryKind::WordBoundary),
        AstNode::StartWord => Some(vm::BoundaryKind::WordStart),
        AstNode::EndWord => Some(vm::BoundaryKind::WordEnd),
        _ => None,
    }
}

fn contains_assertion(nodes: &[AstNode]) -> bool {
    nodes.iter().any(|n| {
        is_zero_width_assertion(n)
            || match n {
                AstNode::Group { nodes, .. } => contains_assertion(nodes),
                AstNode::Alternation(alts) => alts.iter().any(|a| contains_assertion(a)),
                AstNode::ZeroOrMore { node, .. }
                | AstNode::OneOrMore { node, .. }
                | AstNode::Optional { node, .. }
                | AstNode::Exact { node, .. }
                | AstNode::Range { node, .. } => contains_assertion(std::slice::from_ref(node)),
                AstNode::LookAhead { nodes, .. } | AstNode::LookBehind { nodes, .. } => {
                    contains_assertion(nodes)
                }
                _ => false,
            }
    })
}

fn split_boundary_wrapper(
    nodes: &[AstNode],
) -> Option<(Vec<vm::BoundaryKind>, &[AstNode], Vec<vm::BoundaryKind>)> {
    let mut start = 0;
    let mut leading = Vec::new();
    while start < nodes.len() {
        match boundary_kind(&nodes[start]) {
            Some(k) => {
                leading.push(k);
                start += 1;
            }
            None => break,
        }
    }
    let mut end = nodes.len();
    let mut trailing = Vec::new();
    while end > start {
        match boundary_kind(&nodes[end - 1]) {
            Some(k) => {
                trailing.push(k);
                end -= 1;
            }
            None => break,
        }
    }
    trailing.reverse();
    if leading.is_empty() && trailing.is_empty() {
        return None;
    }
    let core = &nodes[start..end];
    if core.is_empty() || contains_assertion(core) {
        return None;
    }
    Some((leading, core, trailing))
}

fn priority_safe(nodes: &[AstNode]) -> bool {
    fn safe(nodes: &[AstNode], nested_variable: bool) -> bool {
        nodes.iter().all(|n| match n {
            AstNode::Alternation(_) => false,
            AstNode::Group { nodes, .. } => safe(nodes, nested_variable),
            AstNode::ZeroOrMore { node, greedy } | AstNode::OneOrMore { node, greedy } => {
                *greedy && !nested_variable && safe(std::slice::from_ref(node), true)
            }
            AstNode::Optional { node, greedy } => {
                *greedy && !nested_variable && safe(std::slice::from_ref(node), true)
            }
            AstNode::Range {
                node,
                greedy,
                min,
                max,
            } => {
                if *max == Some(*min) {
                    // Exact-equivalent: no choice of its own, transparent.
                    safe(std::slice::from_ref(node), nested_variable)
                } else {
                    *greedy && !nested_variable && safe(std::slice::from_ref(node), true)
                }
            }
            AstNode::Exact { node, .. } => safe(std::slice::from_ref(node), nested_variable),
            _ => true,
        })
    }
    safe(nodes, false)
}

fn alternation_of_literals(nodes: &[AstNode]) -> Option<Vec<Box<[u8]>>> {
    let [AstNode::Alternation(alts)] = nodes else {
        return None;
    };
    if alts.is_empty() || alts.len() > 16 {
        return None;
    }
    let mut out = Vec::with_capacity(alts.len());
    for branch in alts {
        let mut bytes = Vec::with_capacity(branch.len());
        for n in branch {
            match n {
                AstNode::Literal(c) if c.is_ascii() => bytes.push(*c as u8),
                _ => return None,
            }
        }
        if bytes.is_empty() {
            return None;
        }
        out.push(bytes.into_boxed_slice());
    }
    Some(out)
}

fn unicode_class_run(nodes: &[AstNode]) -> Option<(CharClass, usize, Option<usize>)> {
    let [node] = nodes else { return None };
    let (inner, min, max, greedy) = match node {
        AstNode::ZeroOrMore { node, greedy } => (node.as_ref(), 0, None, *greedy),
        AstNode::OneOrMore { node, greedy } => (node.as_ref(), 1, None, *greedy),
        AstNode::Optional { node, greedy } => (node.as_ref(), 0, Some(1), *greedy),
        AstNode::Exact { node, count } => (node.as_ref(), *count, Some(*count), true),
        AstNode::Range {
            node,
            min,
            max,
            greedy,
        } => (node.as_ref(), *min, *max, *greedy),
        _ => return None,
    };
    if !greedy {
        return None;
    }
    match inner {
        AstNode::CharClass(c) if !vm::class_is_ascii_only(c) => Some((c.clone(), min, max)),
        _ => None,
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
        for (i, slot) in r.iter_mut().enumerate() {
            *slot = self.0[i] | other.0[i];
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
    let mut active = false;
    for seg in &segs {
        let fixed = seg.min == seg.max;
        if fixed && seg.min == 0 {
            continue; // never consumes anything - transparent
        }
        if active && !seg.alphabet.disjoint(&reachable) {
            return None;
        }
        if fixed {
            // Deterministic: consumed exactly `min`, so the position right
            // after it can't have absorbed anything ambiguously.
            active = false;
        } else if seg.min == 0 {
            reachable = if active {
                reachable.union(&seg.alphabet)
            } else {
                seg.alphabet
            };
            active = true;
        } else {
            reachable = seg.alphabet;
            active = true;
        }
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
        AstNode::OneOrMore { node, .. } => collect_start_bytes(node, ic),
        // See `start_filter_from_class`: `{0}` never actually requires the
        // wrapped node to appear, so it can't license a start-byte filter.
        AstNode::Exact { node, count } if *count > 0 => collect_start_bytes(node, ic),
        AstNode::Group { nodes, .. } => {
            if nodes.is_empty() {
                vec![]
            } else {
                collect_start_bytes(&nodes[0], ic)
            }
        }
        AstNode::Alternation(alts) => {
            let mut bytes = Vec::new();
            for alt in alts {
                let Some(first) = alt.first() else {
                    return vec![];
                };
                let branch_bytes = collect_start_bytes(first, ic);
                if branch_bytes.is_empty() {
                    return vec![];
                }
                bytes.extend(branch_bytes);
            }
            bytes
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
    let Some(first) = nodes.iter().find(|n| !is_zero_width_assertion(n)) else {
        return StartFilter::None;
    };
    let ic = flags.ignore_case.unwrap_or(false);

    // Check if the leading node is a wide class that maps to a byte range.
    if let Some(range_filter) = start_filter_from_class(first) {
        return range_filter;
    }

    let mut bytes = collect_start_bytes(first, ic);
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

fn is_zero_width_assertion(node: &AstNode) -> bool {
    matches!(
        node,
        AstNode::WordBoundary
            | AstNode::StartWord
            | AstNode::EndWord
            | AstNode::StartAnchor
            | AstNode::EndAnchor
            | AstNode::SetMatchStart
            | AstNode::SetMatchEnd
    )
}

/// Map a leading character class to an efficient byte start-filter.
fn start_filter_from_class(node: &AstNode) -> Option<StartFilter> {
    let class = match node {
        AstNode::CharClass(c) => c,
        AstNode::OneOrMore { node, .. } => {
            return start_filter_from_class(node);
        }
        AstNode::Exact { node, count } if *count > 0 => {
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
        Box::new(self.find_all_linear(text))
    }

    fn captures(&self, text: &str) -> Option<Captures> {
        if self.group_count == 0 {
            let full = self.vm.find_raw(text, 0)?;
            return Some(Captures {
                full_match: full,
                groups: vec![],
                named: HashMap::new(),
            });
        }
        let num_slots = 2 * (self.group_count + 1);
        let (full_match, slots) = self.vm.find_captures(text, 0, num_slots)?;
        let groups: Vec<Option<Match>> = (1..=self.group_count)
            .map(|i| {
                match (
                    slots.get(2 * i).copied().flatten(),
                    slots.get(2 * i + 1).copied().flatten(),
                ) {
                    (Some(start), Some(end)) => Some(Match { start, end }),
                    _ => None,
                }
            })
            .collect();
        let named = self
            .named_groups
            .iter()
            .filter_map(|(name, &idx)| {
                groups
                    .get(idx - 1)
                    .cloned()
                    .flatten()
                    .map(|m| (name.clone(), m))
            })
            .collect();
        Some(Captures {
            full_match,
            groups,
            named,
        })
    }

    fn captures_all<'a>(&'a self, text: &'a str) -> Box<dyn Iterator<Item = Captures> + 'a> {
        Box::new(CapturesIterator {
            text,
            regex: self,
            last_end: 0,
            adjacent_empty: Default::default(),
        })
    }

    fn replace(&self, text: &str, replacement: &str) -> String {
        if let Some(caps) = self.captures(text) {
            let m = caps.full_match.clone();
            let mut result = String::with_capacity(text.len());
            result.push_str(&text[..m.start]);
            result.push_str(&crate::captures::expand_replacement(
                &caps,
                replacement,
                text,
            ));
            result.push_str(&text[m.end..]);
            result
        } else {
            text.to_string()
        }
    }

    fn replace_all(&self, text: &str, replacement: &str) -> String {
        let mut result = String::with_capacity(text.len() * 2);
        let mut last_end = 0;
        for caps in self.captures_all(text) {
            let m = caps.full_match.clone();
            result.push_str(&text[last_end..m.start]);
            result.push_str(&crate::captures::expand_replacement(
                &caps,
                replacement,
                text,
            ));
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
            adjacent_empty: Default::default(),
        })
    }
}

enum LinearFindAllInner<'a> {
    Literal {
        inner: memchr::memmem::FindIter<'a, 'a>,
        lit_len: usize,
    },
    LiteralCI(vm::LiteralFindIter<'a, 'a>),
    MultiLiteral(vm::MultiLiteralFindAll<'a, 'a>),
    FixedLen(vm::FixedLenFindAll<'a, 'a>),
    Segments(vm::SegmentsFindAll<'a, 'a>),
    BitParallel(Box<vm::BitParallelFindAll<'a, 'a>>),
    UnicodeClassRun(vm::UnicodeClassRunFindAll<'a, 'a>),
    Nfa(FindMatchesIterator<'a, &'a str>),
    Boundary(BoundaryFilter<'a, Box<LinearFindAllInner<'a>>>),
}

impl<'a> Iterator for LinearFindAllInner<'a> {
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
            Self::MultiLiteral(it) => it.next(),
            Self::FixedLen(it) => it.next(),
            Self::Segments(it) => it.next(),
            Self::BitParallel(it) => it.next(),
            Self::UnicodeClassRun(it) => it.next(),
            Self::Nfa(it) => it.next(),
            Self::Boundary(it) => it.next(),
        }
    }
}

pub struct LinearFindAll<'a> {
    inner: LinearFindAllInner<'a>,
    adjacent_empty: crate::captures::AdjacentEmptyFilter,
}

impl<'a> Iterator for LinearFindAll<'a> {
    type Item = Match;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let m = self.inner.next()?;
            if self.adjacent_empty.should_suppress(m.start, m.end) {
                continue;
            }
            return Some(m);
        }
    }
}

pub struct BoundaryFilter<'a, I> {
    inner: I,
    text: &'a str,
    leading: &'a [vm::BoundaryKind],
    trailing: &'a [vm::BoundaryKind],
}

impl<'a, I: Iterator<Item = Match>> Iterator for BoundaryFilter<'a, I> {
    type Item = Match;

    fn next(&mut self) -> Option<Match> {
        self.inner.by_ref().find(|m| {
            vm::check_all_boundaries(self.leading, &self.text, m.start)
                && vm::check_all_boundaries(self.trailing, &self.text, m.end)
        })
    }
}

pub struct FindMatchesIterator<'a, H: Haystack> {
    pub(crate) text: H,
    pub(crate) regex: &'a LinearRegex,
    pub(crate) last_end: usize,
    pub(crate) adjacent_empty: crate::captures::AdjacentEmptyFilter,
}

impl<'a, H: Haystack> Iterator for FindMatchesIterator<'a, H> {
    type Item = Match;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.last_end > self.text.len() {
                return None;
            }
            let m = self.regex.find_from_at(self.text, self.last_end)?;
            self.last_end = m.end.max(m.start + 1);
            if self.adjacent_empty.should_suppress(m.start, m.end) {
                continue;
            }
            return Some(m);
        }
    }
}

struct CapturesIterator<'a> {
    text: &'a str,
    regex: &'a LinearRegex,
    last_end: usize,
    adjacent_empty: crate::captures::AdjacentEmptyFilter,
}

impl<'a> Iterator for CapturesIterator<'a> {
    type Item = Captures;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
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
            if self
                .adjacent_empty
                .should_suppress(caps.full_match.start, caps.full_match.end)
            {
                continue;
            }
            return Some(caps);
        }
    }
}
