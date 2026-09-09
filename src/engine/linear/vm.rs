use super::nfa::{Nfa, State};
use crate::captures::Match;
use crate::haystack::Haystack;
use crate::parser::CharClass;

// -- Start-byte prefilter -----------------------------------------------------

/// SIMD-backed prefilter: describes which bytes can start a match.
/// `None` means no filter is possible (any byte may start a match).
#[derive(Clone, Debug)]
pub enum StartFilter {
    None,
    One(u8),
    Two(u8, u8),
    Three(u8, u8, u8),
    /// Any byte in the inclusive range [lo, hi].
    ByteRange(u8, u8),
}

impl StartFilter {
    #[inline]
    pub fn has_filter(&self) -> bool {
        !matches!(self, StartFilter::None)
    }

    /// Scan bytes[pos..] for the next potential start position.
    #[inline]
    pub fn find_next_from(&self, bytes: &[u8], pos: usize) -> Option<usize> {
        if pos >= bytes.len() {
            return None;
        }
        let sub = &bytes[pos..];
        let offset = match self {
            StartFilter::None => return Some(pos),
            StartFilter::One(b) => memchr::memchr(*b, sub)?,
            StartFilter::Two(b1, b2) => memchr::memchr2(*b1, *b2, sub)?,
            StartFilter::Three(b1, b2, b3) => memchr::memchr3(*b1, *b2, *b3, sub)?,
            StartFilter::ByteRange(lo, hi) => sub.iter().position(|&b| b >= *lo && b <= *hi)?,
        };
        Some(pos + offset)
    }

    /// O(1) check: could `b` start a match?
    #[inline]
    pub fn matches_byte(&self, b: u8) -> bool {
        match self {
            StartFilter::None => true,
            StartFilter::One(b1) => b == *b1,
            StartFilter::Two(b1, b2) => b == *b1 || b == *b2,
            StartFilter::Three(b1, b2, b3) => b == *b1 || b == *b2 || b == *b3,
            StartFilter::ByteRange(lo, hi) => b >= *lo && b <= *hi,
        }
    }
}

// -- Boundary-wrapper postcondition -------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryKind {
    WordBoundary,
    WordStart,
    WordEnd,
}

impl BoundaryKind {
    #[inline]
    fn check<H: Haystack>(self, text: &H, pos: usize) -> bool {
        match self {
            BoundaryKind::WordBoundary => is_word_boundary(text, pos),
            BoundaryKind::WordStart => is_word_start(text, pos),
            BoundaryKind::WordEnd => is_word_end(text, pos),
        }
    }
}

/// All of `kinds` must hold at `pos` (an empty slice trivially passes).
#[inline]
pub fn check_all_boundaries<H: Haystack>(kinds: &[BoundaryKind], text: &H, pos: usize) -> bool {
    kinds.iter().all(|k| k.check(text, pos))
}

// -- Multi-literal fast path (alternation of literals) ------------------------

pub struct MultiLiteral {
    literals: Vec<Box<[u8]>>,
    case_insensitive: bool,
    start_filter: StartFilter,
}

impl MultiLiteral {
    pub fn new(literals: Vec<Box<[u8]>>, case_insensitive: bool) -> Self {
        let mut first_bytes: Vec<u8> = literals
            .iter()
            .flat_map(|lit| {
                let b = lit[0];
                if case_insensitive {
                    vec![b.to_ascii_lowercase(), b.to_ascii_uppercase()]
                } else {
                    vec![b]
                }
            })
            .collect();
        first_bytes.sort_unstable();
        first_bytes.dedup();
        let start_filter = match first_bytes.len() {
            0 => StartFilter::None,
            1 => StartFilter::One(first_bytes[0]),
            2 => StartFilter::Two(first_bytes[0], first_bytes[1]),
            3 => StartFilter::Three(first_bytes[0], first_bytes[1], first_bytes[2]),
            _ => StartFilter::None,
        };
        Self {
            literals,
            case_insensitive,
            start_filter,
        }
    }

    /// Length of whichever branch matches at `pos`, if any.
    #[inline]
    fn matches_at(&self, bytes: &[u8], pos: usize) -> Option<usize> {
        for lit in &self.literals {
            let end = pos + lit.len();
            if end > bytes.len() {
                continue;
            }
            let cand = &bytes[pos..end];
            let matched = if self.case_insensitive {
                cand.eq_ignore_ascii_case(lit)
            } else {
                cand == lit.as_ref()
            };
            if matched {
                return Some(lit.len());
            }
        }
        None
    }

    pub fn find_in(&self, bytes: &[u8], start: usize) -> Option<Match> {
        let mut pos = start;
        loop {
            pos = self.start_filter.find_next_from(bytes, pos)?;
            if let Some(len) = self.matches_at(bytes, pos) {
                return Some(Match {
                    start: pos,
                    end: pos + len,
                });
            }
            pos += 1;
        }
    }
}

pub struct MultiLiteralFindAll<'h, 'v> {
    ml: &'v MultiLiteral,
    bytes: &'h [u8],
    pos: usize,
}

impl<'h, 'v> MultiLiteralFindAll<'h, 'v> {
    fn new(ml: &'v MultiLiteral, bytes: &'h [u8], start: usize) -> Self {
        Self {
            ml,
            bytes,
            pos: start,
        }
    }
}

impl<'h, 'v> Iterator for MultiLiteralFindAll<'h, 'v> {
    type Item = Match;

    #[inline]
    fn next(&mut self) -> Option<Match> {
        let m = self.ml.find_in(self.bytes, self.pos)?;
        self.pos = m.end.max(m.start + 1);
        Some(m)
    }
}

// -- Unicode single-class-run fast path ---------------------------------------

pub struct UnicodeClassRun {
    class: CharClass,
    ignore_case: bool,
    dotall: bool,
    min: usize,
    max: Option<usize>,
}

impl UnicodeClassRun {
    pub fn new(
        class: CharClass,
        ignore_case: bool,
        dotall: bool,
        min: usize,
        max: Option<usize>,
    ) -> Self {
        Self {
            class,
            ignore_case,
            dotall,
            min,
            max,
        }
    }

    pub fn find_in(&self, bytes: &[u8], start: usize) -> Option<Match> {
        if self.min == 0 {
            if start > bytes.len() {
                return None;
            }
            let mut count = 0usize;
            let mut p = start;
            while self.max.is_none_or(|m| count < m) && p < bytes.len() {
                let (c, len) = read_char(bytes, p);
                if !matches_class_static(&self.class, c, self.ignore_case, self.dotall) {
                    break;
                }
                count += 1;
                p += len;
            }
            return Some(Match { start, end: p });
        }
        let mut pos = start;
        while pos < bytes.len() {
            let (c, len) = read_char(bytes, pos);
            if !matches_class_static(&self.class, c, self.ignore_case, self.dotall) {
                pos += len;
                continue;
            }
            let match_start = pos;
            let mut count = 1usize;
            let mut p = pos + len;
            while self.max.is_none_or(|m| count < m) && p < bytes.len() {
                let (c2, len2) = read_char(bytes, p);
                if !matches_class_static(&self.class, c2, self.ignore_case, self.dotall) {
                    break;
                }
                count += 1;
                p += len2;
            }
            if count >= self.min {
                return Some(Match {
                    start: match_start,
                    end: p,
                });
            }
            pos = p;
        }
        None
    }
}

pub struct UnicodeClassRunFindAll<'h, 'v> {
    run: &'v UnicodeClassRun,
    bytes: &'h [u8],
    pos: usize,
}

impl<'h, 'v> UnicodeClassRunFindAll<'h, 'v> {
    fn new(run: &'v UnicodeClassRun, bytes: &'h [u8], start: usize) -> Self {
        Self {
            run,
            bytes,
            pos: start,
        }
    }
}

impl<'h, 'v> Iterator for UnicodeClassRunFindAll<'h, 'v> {
    type Item = Match;

    #[inline]
    fn next(&mut self) -> Option<Match> {
        let m = self.run.find_in(self.bytes, self.pos)?;
        self.pos = m.end.max(m.start + 1);
        Some(m)
    }
}

// -- Literal fast path --------------------------------------------------------

/// Precomputed literal for bypassing NFA simulation entirely.
#[derive(Clone, Debug)]
pub struct Literal {
    /// Bytes of the literal (lowercased for CI patterns).
    pub bytes: Box<[u8]>,
    /// If true, match case-insensitively (ASCII only).
    pub case_insensitive: bool,
    pub finder: memchr::memmem::Finder<'static>,
}

impl Literal {
    pub fn new(bytes: Box<[u8]>, case_insensitive: bool) -> Self {
        let finder = memchr::memmem::Finder::new(&bytes).into_owned();
        Self {
            bytes,
            case_insensitive,
            finder,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn find_in(&self, haystack: &[u8], start: usize) -> Option<usize> {
        if start >= haystack.len() {
            return None;
        }
        if !self.case_insensitive {
            // Case-sensitive: reuse the prebuilt SIMD Finder.
            self.finder.find(&haystack[start..]).map(|i| i + start)
        } else {
            self.find_in_ci(haystack, start)
        }
    }

    fn find_in_ci(&self, haystack: &[u8], start: usize) -> Option<usize> {
        find_ci_from(haystack, &self.bytes, &self.finder, start)
    }
}

#[inline]
fn find_ci_from(
    haystack: &[u8],
    needle_lower: &[u8],
    finder: &memchr::memmem::Finder<'static>,
    start: usize,
) -> Option<usize> {
    let _ = finder; // only used by the non-x86_64 fallback below
    if needle_lower.is_empty() {
        return Some(start);
    }
    if needle_lower.len() == 1 {
        return naive_ci_find(haystack, needle_lower, start);
    }
    #[cfg(target_arch = "x86_64")]
    {
        simd_ci::find_from(haystack, needle_lower, start)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        if haystack.len().saturating_sub(start) <= CI_SMALL_THRESHOLD {
            naive_ci_find(haystack, needle_lower, start)
        } else {
            let mut buf: Vec<u8> = Vec::with_capacity(CI_CHUNK + needle_lower.len());
            ci_chunked_search(finder, needle_lower.len(), haystack, start, &mut buf)
                .map(|(abs, _rel, _core_len, _chunk_base)| abs)
        }
    }
}

#[cfg(not(target_arch = "x86_64"))]
const CI_CHUNK: usize = 16384;

#[cfg(not(target_arch = "x86_64"))]
const CI_SMALL_THRESHOLD: usize = 4096;

#[cfg(target_arch = "x86_64")]
pub(crate) mod simd_ci {
    use std::arch::x86_64::*;

    pub(super) fn find_from(haystack: &[u8], needle_lower: &[u8], start: usize) -> Option<usize> {
        FindIter::new(haystack, needle_lower, start)?.next()
    }

    pub(crate) enum FindIter<'h, 'n> {
        Avx2(Avx2FindIter<'h, 'n>),
        Sse2(Sse2FindIter<'h, 'n>),
    }

    impl<'h, 'n> FindIter<'h, 'n> {
        /// Returns `None` if `needle_lower` is too short for this path
        /// (length < 2) - callers fall back to `naive_ci_find` for that case.
        pub(super) fn new(
            haystack: &'h [u8],
            needle_lower: &'n [u8],
            start: usize,
        ) -> Option<Self> {
            if needle_lower.len() < 2 {
                return None;
            }
            if is_x86_feature_detected!("avx2") {
                Some(FindIter::Avx2(Avx2FindIter::new(
                    haystack,
                    needle_lower,
                    start,
                )))
            } else {
                Some(FindIter::Sse2(Sse2FindIter::new(
                    haystack,
                    needle_lower,
                    start,
                )))
            }
        }

        pub(super) fn next(&mut self) -> Option<usize> {
            match self {
                FindIter::Avx2(it) => it.next(),
                FindIter::Sse2(it) => it.next(),
            }
        }
    }

    macro_rules! ci_simd_kernel {
        ($name:ident, $feature:literal, $vec:ty, $lanes:expr, $set1:ident, $loadu:ident, $cmpeq:ident, $or:ident, $and:ident, $movemask:ident, $next_fn:ident) => {
            pub(crate) struct $name<'h, 'n> {
                haystack: &'h [u8],
                needle_lower: &'n [u8],
                pos: usize,
                v_first_lo: $vec,
                v_first_up: $vec,
                v_last_lo: $vec,
                v_last_up: $vec,
            }

            impl<'h, 'n> $name<'h, 'n> {
                /// Caller must have already checked `needle_lower.len() >= 2`.
                fn new(haystack: &'h [u8], needle_lower: &'n [u8], start: usize) -> Self {
                    let first = needle_lower[0];
                    let last = needle_lower[needle_lower.len() - 1];
                    let first_up = first.to_ascii_uppercase();
                    let last_up = last.to_ascii_uppercase();
                    let (v_first_lo, v_first_up, v_last_lo, v_last_up) = unsafe {
                        (
                            $set1(first as i8),
                            $set1(first_up as i8),
                            $set1(last as i8),
                            $set1(last_up as i8),
                        )
                    };
                    Self {
                        haystack,
                        needle_lower,
                        pos: start,
                        v_first_lo,
                        v_first_up,
                        v_last_lo,
                        v_last_up,
                    }
                }

                fn next(&mut self) -> Option<usize> {
                    // SAFETY: see the safety note on `new` above.
                    unsafe { self.$next_fn() }
                }

                #[target_feature(enable = $feature)]
                unsafe fn $next_fn(&mut self) -> Option<usize> {
                    let haystack = self.haystack;
                    let needle_lower = self.needle_lower;
                    let n = haystack.len();
                    let l = needle_lower.len();
                    let mut pos = self.pos;

                    let simd_limit = n.checked_sub(l - 1 + $lanes);

                    while let Some(limit) = simd_limit
                        && pos <= limit
                    {
                        let (chunk_first, chunk_last) = unsafe {
                            (
                                $loadu(haystack.as_ptr().add(pos) as *const $vec),
                                $loadu(haystack.as_ptr().add(pos + l - 1) as *const $vec),
                            )
                        };
                        let eq_first = $or(
                            $cmpeq(chunk_first, self.v_first_lo),
                            $cmpeq(chunk_first, self.v_first_up),
                        );
                        let eq_last = $or(
                            $cmpeq(chunk_last, self.v_last_lo),
                            $cmpeq(chunk_last, self.v_last_up),
                        );
                        let mut mask = $movemask($and(eq_first, eq_last)) as u32;

                        while mask != 0 {
                            let bit = mask.trailing_zeros() as usize;
                            mask &= mask - 1;
                            let cand = pos + bit;
                            if verify(haystack, needle_lower, cand) {
                                self.pos = cand + l;
                                return Some(cand);
                            }
                        }
                        pos += $lanes;
                    }

                    // Scalar tail: fewer than $lanes + (l - 1) bytes remain.
                    while pos + l <= n {
                        if verify(haystack, needle_lower, pos) {
                            self.pos = pos + l;
                            return Some(pos);
                        }
                        pos += 1;
                    }
                    self.pos = pos;
                    None
                }
            }
        };
    }

    ci_simd_kernel!(
        Sse2FindIter,
        "sse2",
        __m128i,
        16,
        _mm_set1_epi8,
        _mm_loadu_si128,
        _mm_cmpeq_epi8,
        _mm_or_si128,
        _mm_and_si128,
        _mm_movemask_epi8,
        next_sse2
    );
    ci_simd_kernel!(
        Avx2FindIter,
        "avx2",
        __m256i,
        32,
        _mm256_set1_epi8,
        _mm256_loadu_si256,
        _mm256_cmpeq_epi8,
        _mm256_or_si256,
        _mm256_and_si256,
        _mm256_movemask_epi8,
        next_avx2
    );

    #[inline]
    fn verify(haystack: &[u8], needle_lower: &[u8], pos: usize) -> bool {
        haystack[pos..pos + needle_lower.len()]
            .iter()
            .zip(needle_lower)
            .all(|(&h, &nb)| h.to_ascii_lowercase() == nb)
    }
}

fn naive_ci_find(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if start >= haystack.len() {
        return None;
    }
    let first = needle[0];
    let first_upper = first.to_ascii_uppercase();
    let mut pos = start;
    loop {
        if pos >= haystack.len() {
            return None;
        }
        let sub = &haystack[pos..];
        let idx = if first != first_upper {
            memchr::memchr2(first, first_upper, sub)?
        } else {
            memchr::memchr(first, sub)?
        };
        let abs = pos + idx;
        let end = abs + needle.len();
        if end > haystack.len() {
            return None;
        }
        if haystack[abs..end]
            .iter()
            .zip(needle)
            .all(|(&tb, &pb)| tb.to_ascii_lowercase() == pb)
        {
            return Some(abs);
        }
        pos = abs + 1;
    }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn ci_chunked_search(
    finder: &memchr::memmem::Finder<'static>,
    needle_len: usize,
    haystack: &[u8],
    start: usize,
    buf: &mut Vec<u8>,
) -> Option<(usize, usize, usize, usize)> {
    let mut chunk_base = start;
    while chunk_base < haystack.len() {
        let core_end = (chunk_base + CI_CHUNK).min(haystack.len());
        let ext_end = (core_end + needle_len - 1).min(haystack.len());
        buf.clear();
        buf.extend(
            haystack[chunk_base..ext_end]
                .iter()
                .map(u8::to_ascii_lowercase),
        );
        let core_len = core_end - chunk_base;
        if let Some(rel) = finder.find(buf)
            && rel < core_len
        {
            return Some((chunk_base + rel, rel, core_len, chunk_base));
        }
        chunk_base = core_end;
    }
    None
}

#[allow(private_interfaces)]
#[allow(clippy::large_enum_variant)]
pub enum LiteralFindIter<'h, 'n> {
    CaseSensitive {
        /// Streams through `haystack[start..]` yielding match offsets relative to `offset`.
        inner: memchr::memmem::FindIter<'h, 'n>,
        /// Absolute start position so returned Match positions are haystack-relative.
        offset: usize,
        lit_len: usize,
    },
    CaseInsensitive {
        haystack: &'h [u8],
        pos: usize,
        lit: &'n Literal,
        #[cfg(target_arch = "x86_64")]
        simd_iter: Option<simd_ci::FindIter<'h, 'n>>,
    },
}

impl<'h, 'n> LiteralFindIter<'h, 'n> {
    pub fn new(haystack: &'h [u8], lit: &'n Literal, start: usize) -> Self {
        if !lit.case_insensitive {
            LiteralFindIter::CaseSensitive {
                inner: lit.finder.find_iter(&haystack[start..]),
                offset: start,
                lit_len: lit.len(),
            }
        } else {
            LiteralFindIter::CaseInsensitive {
                haystack,
                pos: start,
                lit,
                #[cfg(target_arch = "x86_64")]
                simd_iter: simd_ci::FindIter::new(haystack, &lit.bytes, start),
            }
        }
    }
}

impl<'h, 'n> Iterator for LiteralFindIter<'h, 'n> {
    type Item = Match;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            LiteralFindIter::CaseSensitive {
                inner,
                offset,
                lit_len,
            } => {
                let idx = inner.next()?;
                let start = *offset + idx;
                Some(Match {
                    start,
                    end: start + *lit_len,
                })
            }
            LiteralFindIter::CaseInsensitive {
                haystack,
                pos,
                lit,
                #[cfg(target_arch = "x86_64")]
                simd_iter,
            } => {
                #[cfg(target_arch = "x86_64")]
                if let Some(iter) = simd_iter {
                    let start = iter.next()?;
                    return Some(Match {
                        start,
                        end: start + lit.len(),
                    });
                }
                if lit.bytes.is_empty() {
                    return None;
                }
                let start = find_ci_from(haystack, &lit.bytes, &lit.finder, *pos)?;
                let end = start + lit.len();
                *pos = end;
                Some(Match { start, end })
            }
        }
    }
}

// -- Thread list --------------------------------------------------------------

/// Generation-tagged thread set with O(1) insert, contains, and origin lookup.
/// `states` stores only PCs (8 bytes each); origins live in `seen_origin[pc]`.
struct ThreadList {
    /// Active PCs this generation, in insertion order.
    states: Vec<usize>,
    /// seen_gen[pc] == generation iff state `pc` is active this round.
    seen_gen: Vec<u32>,
    /// seen_origin[pc] stores the match origin of state `pc` (valid when seen_gen[pc]==gen).
    seen_origin: Vec<usize>,
    generation: u32,
}

impl ThreadList {
    fn new(capacity: usize) -> Self {
        Self {
            states: Vec::with_capacity(capacity),
            seen_gen: vec![0u32; capacity],
            seen_origin: vec![0usize; capacity],
            generation: 1,
        }
    }

    /// O(1) clear via generation bump.
    #[inline]
    fn clear(&mut self) {
        self.states.clear();
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.generation = 1;
        }
    }

    #[inline(always)]
    fn contains(&self, pc: usize) -> bool {
        self.seen_gen[pc] == self.generation
    }

    /// Insert state at `pc` with `origin`. First insertion wins (earliest origin).
    #[inline(always)]
    fn insert(&mut self, pc: usize, origin: usize) {
        if self.seen_gen[pc] != self.generation {
            self.seen_gen[pc] = self.generation;
            self.seen_origin[pc] = origin;
            self.states.push(pc);
        }
    }

    #[inline(always)]
    fn position_of(&self, pc: usize) -> Option<usize> {
        if self.seen_gen[pc] == self.generation {
            self.states.iter().position(|&s| s == pc)
        } else {
            None
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

/// Per-thread payload for `CaptureThreadList`: the match origin (needed for
/// leftmost resolution, same as `ThreadList`) plus a full capture-slot array.
struct CaptureThread {
    origin: usize,
    slots: Vec<Option<usize>>,
}

struct CaptureThreadList {
    states: Vec<usize>,
    seen_gen: Vec<u32>,
    data: Vec<CaptureThread>,
    generation: u32,
}

impl CaptureThreadList {
    fn new(num_states: usize, num_slots: usize) -> Self {
        Self {
            states: Vec::with_capacity(num_states),
            seen_gen: vec![0u32; num_states],
            data: (0..num_states)
                .map(|_| CaptureThread {
                    origin: 0,
                    slots: vec![None; num_slots],
                })
                .collect(),
            generation: 1,
        }
    }

    #[inline]
    fn clear(&mut self) {
        self.states.clear();
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.generation = 1;
        }
    }

    #[inline(always)]
    fn contains(&self, pc: usize) -> bool {
        self.seen_gen[pc] == self.generation
    }

    /// Insert state at `pc` with `origin`/`slots`. First insertion wins (see
    /// `add_epsilon_captures`'s priority ordering).
    fn insert(&mut self, pc: usize, origin: usize, slots: &[Option<usize>]) {
        if self.seen_gen[pc] != self.generation {
            self.seen_gen[pc] = self.generation;
            self.data[pc].origin = origin;
            self.data[pc].slots.copy_from_slice(slots);
            self.states.push(pc);
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// See `ThreadList::position_of` - same purpose, same reasoning.
    #[inline(always)]
    fn position_of(&self, pc: usize) -> Option<usize> {
        if self.seen_gen[pc] == self.generation {
            self.states.iter().position(|&s| s == pc)
        } else {
            None
        }
    }
}

// -- VM context ---------------------------------------------------------------

struct VMContext {
    current: ThreadList,
    next: ThreadList,
    epsilon_stack: Vec<(usize, usize)>,
}

impl VMContext {
    fn new(num_states: usize) -> Self {
        Self {
            current: ThreadList::new(num_states),
            next: ThreadList::new(num_states),
            epsilon_stack: Vec::with_capacity(num_states),
        }
    }

    #[inline]
    fn reset(&mut self) {
        self.current.clear();
        self.next.clear();
        self.epsilon_stack.clear();
    }
}

// -- Bit-parallel NFA ---------------------------------------------------------

/// Precomputed table enabling bit-parallel NFA simulation without per-char stack ops.
/// Only built for patterns with <= 64 NFA states and no zero-width assertions.
pub struct BitParallelNfa {
    char_transitions: Box<[u64]>,
    /// Epsilon closure of the start state (the initial DFA state).
    initial: u64,
    /// Bit position of the NFA match state.
    match_bit: usize,
}

impl BitParallelNfa {
    pub fn build(nfa: &Nfa) -> Option<Self> {
        Self::build_inner(nfa, false)
    }

    pub fn build_ascii_gated(nfa: &Nfa) -> Option<Self> {
        Self::build_inner(nfa, true)
    }

    fn build_inner(nfa: &Nfa, ascii_gated: bool) -> Option<Self> {
        let n = nfa.states.len();
        if n > 64 {
            return None;
        }

        for s in &nfa.states {
            match s {
                // Zero-width assertions make the epsilon closure position-dependent.
                State::AnchorStart(_)
                | State::AnchorEnd(_)
                | State::WordBoundary(_)
                | State::WordStart(_)
                | State::WordEnd(_) => return None,
                State::Char(c, _) if !c.is_ascii() => return None,
                State::Any(_) if !ascii_gated => return None,
                State::Class(class, _) if !ascii_gated && !class_is_ascii_only(class) => {
                    return None;
                }
                _ => {}
            }
        }

        // Precompute epsilon closures for every state.
        let eps: Vec<u64> = (0..n).map(|s| eps_closure(s, nfa)).collect();

        // Build the flat char_transitions table (single allocation, row = 256 u64s).
        let ic = nfa.flags.ignore_case.unwrap_or(false);
        let dotall = nfa.flags.dotall;
        let mut table = vec![0u64; n * 256];

        for state in 0..n {
            let row = state * 256;
            match &nfa.states[state] {
                State::Char(c, next) => {
                    if c.is_ascii() {
                        let b = *c as u8;
                        table[row + b as usize] |= eps[*next];
                        if ic {
                            let lo = b.to_ascii_lowercase();
                            let up = b.to_ascii_uppercase();
                            if lo != up {
                                table[row + lo as usize] |= eps[*next];
                                table[row + up as usize] |= eps[*next];
                            }
                        }
                    }
                    // Non-ASCII Char: skip (leaves entry as 0, PikeVM fallback handles it)
                }
                State::Class(class, next) => {
                    let dest = eps[*next];
                    for byte in 0u8..=127u8 {
                        let c = byte as char;
                        if matches_class_static(class, c, ic, dotall) {
                            table[row + byte as usize] |= dest;
                        }
                    }
                }
                State::Any(next) => {
                    let dest = eps[*next];
                    for byte in 0u8..=127u8 {
                        if byte != b'\n' || dotall {
                            table[row + byte as usize] |= dest;
                        }
                    }
                }
                // Epsilon / terminal states: no consuming transition.
                _ => {}
            }
        }

        let initial = eps[nfa.start];

        Some(BitParallelNfa {
            char_transitions: table.into_boxed_slice(),
            initial,
            match_bit: nfa.match_state,
        })
    }
}

/// Compute the epsilon closure of `state` as a bitmask.
fn eps_closure(start: usize, nfa: &Nfa) -> u64 {
    let mut mask = 0u64;
    let mut stack = vec![start];
    while let Some(s) = stack.pop() {
        if mask & (1u64 << s) != 0 {
            continue;
        }
        mask |= 1u64 << s;
        match nfa.states[s] {
            State::Jump(next) => stack.push(next),
            State::Split(s1, s2) => {
                stack.push(s1);
                stack.push(s2);
            }
            State::Save(_, next) => stack.push(next),
            // Consuming and terminal states stop epsilon propagation.
            _ => {}
        }
    }
    mask
}

// -- Lazy DFA cache (persistent across find calls) ----------------------------

const DFA_CACHE_SIZE: usize = 1024;

struct LazyDfaCache {
    keys: [u64; DFA_CACHE_SIZE], // bitmask key (u64::MAX = empty)
    byte: [u8; DFA_CACHE_SIZE],  // byte key
    val: [u64; DFA_CACHE_SIZE],  // cached next bitmask
}

impl LazyDfaCache {
    fn new() -> Self {
        Self {
            keys: [u64::MAX; DFA_CACHE_SIZE],
            byte: [0; DFA_CACHE_SIZE],
            val: [0; DFA_CACHE_SIZE],
        }
    }
}

// -- PikeVM -------------------------------------------------------------------

use std::sync::Mutex;

pub struct PikeVM {
    nfa: Nfa,
    start_filter: StartFilter,
    /// If Some, the entire pattern is a literal - skip NFA simulation.
    literal: Option<Literal>,
    /// Bit-parallel NFA for fast simulation (None if NFA too large or has assertions).
    bp_nfa: Option<BitParallelNfa>,
    bp_nfa_ascii: Option<BitParallelNfa>,
    ctx: Mutex<VMContext>,
    dfa_cache: Mutex<LazyDfaCache>,
    ascii_cache: Mutex<(usize, usize, bool)>,
    fixed_len: Option<usize>,
    segments: Option<Vec<super::Segment>>,
    leading_boundary: Vec<BoundaryKind>,
    trailing_boundary: Vec<BoundaryKind>,
    multi_literal: Option<MultiLiteral>,
    unicode_class_run: Option<UnicodeClassRun>,
}

impl PikeVM {
    pub fn new(
        nfa: Nfa,
        start_filter: StartFilter,
        literal: Option<Literal>,
        fixed_len: Option<usize>,
        segments: Option<Vec<super::Segment>>,
        priority_safe: bool,
    ) -> Self {
        let num_states = nfa.states.len();
        let bp_nfa = if literal.is_none() && priority_safe {
            BitParallelNfa::build(&nfa)
        } else {
            None // literal path bypasses NFA entirely
        };
        // Only worth building the ASCII-gated table when the strict one
        // failed and there's no literal fast path already covering it.
        let bp_nfa_ascii = if literal.is_none() && bp_nfa.is_none() && priority_safe {
            BitParallelNfa::build_ascii_gated(&nfa)
        } else {
            None
        };
        Self {
            nfa,
            start_filter,
            literal,
            bp_nfa,
            bp_nfa_ascii,
            ctx: Mutex::new(VMContext::new(num_states)),
            dfa_cache: Mutex::new(LazyDfaCache::new()),
            ascii_cache: Mutex::new((0, 0, false)),
            fixed_len,
            segments,
            leading_boundary: Vec::new(),
            trailing_boundary: Vec::new(),
            multi_literal: None,
            unicode_class_run: None,
        }
    }

    pub fn with_boundary(
        mut self,
        leading: Vec<BoundaryKind>,
        trailing: Vec<BoundaryKind>,
    ) -> Self {
        self.leading_boundary = leading;
        self.trailing_boundary = trailing;
        self
    }

    /// Attach the multi-literal fast path (see `MultiLiteral`).
    pub fn with_multi_literal(mut self, ml: MultiLiteral) -> Self {
        self.multi_literal = Some(ml);
        self
    }

    /// Attach the Unicode single-class-run fast path (see `UnicodeClassRun`).
    pub fn with_unicode_class_run(mut self, run: UnicodeClassRun) -> Self {
        self.unicode_class_run = Some(run);
        self
    }

    #[inline]
    pub fn has_boundary(&self) -> bool {
        !self.leading_boundary.is_empty() || !self.trailing_boundary.is_empty()
    }

    #[inline]
    pub fn leading_boundary(&self) -> &[BoundaryKind] {
        &self.leading_boundary
    }

    #[inline]
    pub fn trailing_boundary(&self) -> &[BoundaryKind] {
        &self.trailing_boundary
    }

    pub fn multi_literal_find_all<'h, 'vm>(
        &'vm self,
        bytes: &'h [u8],
        start: usize,
    ) -> Option<MultiLiteralFindAll<'h, 'vm>> {
        self.multi_literal
            .as_ref()
            .map(|ml| MultiLiteralFindAll::new(ml, bytes, start))
    }

    /// Returns a single-pass iterator for the Unicode single-class-run fast
    /// path (see `UnicodeClassRun`). `None` if the pattern doesn't qualify.
    pub fn unicode_class_run_find_all<'h, 'vm>(
        &'vm self,
        bytes: &'h [u8],
        start: usize,
    ) -> Option<UnicodeClassRunFindAll<'h, 'vm>> {
        self.unicode_class_run
            .as_ref()
            .map(|run| UnicodeClassRunFindAll::new(run, bytes, start))
    }

    /// `bytes.is_ascii()`, memoised by buffer identity (see `ascii_cache`).
    #[inline]
    fn is_ascii_cached(&self, bytes: &[u8]) -> bool {
        let key = (bytes.as_ptr() as usize, bytes.len());
        let mut cache = self.ascii_cache.lock().unwrap();
        let (ptr, len, val) = *cache;
        if (ptr, len) == key {
            return val;
        }
        let val = bytes.is_ascii();
        *cache = (key.0, key.1, val);
        val
    }

    pub fn find_from<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        self.find_raw(text, start_index)
    }

    #[inline]
    fn bp_table_for<'v>(&'v self, bytes: &[u8]) -> Option<&'v BitParallelNfa> {
        if let Some(bp) = &self.bp_nfa {
            Some(bp)
        } else if let Some(bp) = &self.bp_nfa_ascii
            && self.is_ascii_cached(bytes)
        {
            Some(bp)
        } else {
            None
        }
    }

    pub fn find_all_bitparallel<'v, 'h>(
        &'v self,
        bytes: &'h [u8],
        start: usize,
    ) -> Option<BitParallelFindAll<'v, 'h>> {
        let bp = self.bp_table_for(bytes)?;
        Some(BitParallelFindAll::new(self, bp, bytes, start))
    }

    pub fn find_all_segments<'v, 'h>(
        &'v self,
        bytes: &'h [u8],
        start: usize,
    ) -> Option<SegmentsFindAll<'h, 'v>> {
        let segs = self.segments.as_deref()?;
        if !self.is_ascii_cached(bytes) {
            return None;
        }
        Some(SegmentsFindAll::new(segs, bytes, start))
    }

    /// Access the precomputed literal, if any.
    #[inline]
    pub fn literal(&self) -> Option<&Literal> {
        self.literal.as_ref()
    }

    /// The statically-known fixed match length, if any (see `fixed_length`).
    #[inline]
    pub fn fixed_len(&self) -> Option<usize> {
        self.fixed_len
    }

    #[inline]
    pub fn segments(&self) -> Option<&[super::Segment]> {
        self.segments.as_deref()
    }

    /// Returns a single-pass iterator for pure-literal patterns on contiguous bytes.
    /// Returns `None` if the pattern is not a pure literal (fall back to NFA).
    pub fn literal_find_all<'h, 'vm>(
        &'vm self,
        bytes: &'h [u8],
        start: usize,
    ) -> Option<LiteralFindIter<'h, 'vm>> {
        // `lit` borrows from `self` with lifetime `'vm`; the finder inside is
        // also `'vm`-lived, giving `LiteralFindIter<'h, 'vm>`.
        self.literal
            .as_ref()
            .map(|lit| LiteralFindIter::new(bytes, lit, start))
    }

    #[inline]
    pub fn find_raw<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        if !self.has_boundary() {
            return self.find_raw_unwrapped(text, start_index);
        }
        let mut start = start_index;
        loop {
            let m = self.find_raw_unwrapped(text, start)?;
            if check_all_boundaries(&self.leading_boundary, &text, m.start)
                && check_all_boundaries(&self.trailing_boundary, &text, m.end)
            {
                return Some(m);
            }
            start = m.end.max(m.start + 1);
        }
    }

    #[inline]
    fn find_raw_unwrapped<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        if let Some(bytes) = text.as_bytes_opt() {
            // Contiguous bytes: use fast paths in priority order.
            if let Some(ml) = &self.multi_literal {
                return ml.find_in(bytes, start_index);
            }
            if let Some(lit) = &self.literal {
                let pos = lit.find_in(bytes, start_index)?;
                return Some(Match {
                    start: pos,
                    end: pos + lit.len(),
                });
            }
            if let Some(fixed_len) = self.fixed_len
                && let Some(bp) = self.bp_table_for(bytes)
            {
                return self.find_raw_fixed_len(bp, bytes, start_index, fixed_len);
            }
            if let Some(segs) = &self.segments
                && self.is_ascii_cached(bytes)
            {
                return find_via_segments(segs, bytes, start_index);
            }
            // Bit-parallel NFA: replaces add_epsilon stack with table lookups.
            if let Some(bp) = &self.bp_nfa {
                return self.find_raw_bitparallel(bp, bytes, start_index);
            }
            if let Some(bp) = &self.bp_nfa_ascii
                && self.is_ascii_cached(bytes)
            {
                return self.find_raw_bitparallel(bp, bytes, start_index);
            }
            if let Some(run) = &self.unicode_class_run {
                return run.find_in(bytes, start_index);
            }
            if self.start_filter.has_filter() {
                return self.find_raw_prefilter(text, bytes, start_index);
            }
            self.find_raw_baseline(text, bytes, start_index)
        } else {
            self.find_raw_char(text, start_index)
        }
    }

    // -- Fixed-length execution -------------------------------------------------

    #[inline(always)]
    fn find_raw_fixed_len(
        &self,
        bp: &BitParallelNfa,
        bytes: &[u8],
        start_index: usize,
        fixed_len: usize,
    ) -> Option<Match> {
        let n = bytes.len();
        let match_mask = 1u64 << bp.match_bit;
        let tbl = &bp.char_transitions;
        let mut current: u64 = 0;
        let mut pos = start_index;

        let mut cache_guard = self.dfa_cache.lock().unwrap();
        let cache = &mut *cache_guard;
        let ck = &mut cache.keys;
        let cb = &mut cache.byte;
        let cv = &mut cache.val;

        if self.start_filter.has_filter() {
            match self.start_filter.find_next_from(bytes, pos) {
                Some(p) => pos = p,
                None => return None,
            }
        }

        while pos <= n {
            current |= bp.initial;

            if current & match_mask != 0 {
                return Some(Match {
                    start: pos - fixed_len,
                    end: pos,
                });
            }

            if pos == n {
                break;
            }

            let byte = bytes[pos];

            let slot = (current
                .wrapping_mul(0x9e3779b97f4a7c15)
                .wrapping_add((byte as u64).wrapping_mul(0x517cc1b727220a95))
                >> 54) as usize
                & (DFA_CACHE_SIZE - 1);
            current = if ck[slot] == current && cb[slot] == byte {
                cv[slot]
            } else {
                let mut nxt = 0u64;
                let mut active = current;
                while active != 0 {
                    let i = active.trailing_zeros() as usize;
                    active &= active - 1;
                    // SAFETY: i < n_states <= 64; byte is a valid u8 index (0-255).
                    nxt |= unsafe { *tbl.get_unchecked(i * 256 + byte as usize) };
                }
                ck[slot] = current;
                cb[slot] = byte;
                cv[slot] = nxt;
                nxt
            };

            pos += 1;

            if current == 0 && self.start_filter.has_filter() {
                match self.start_filter.find_next_from(bytes, pos) {
                    Some(p) => pos = p,
                    None => break,
                }
            }
        }

        None
    }

    /// Streaming find-all entry point for the fixed-length fast path (see
    /// `find_raw_fixed_len`). Returns `None` if it doesn't apply right now.
    pub fn find_all_fixed_len<'v, 'h>(
        &'v self,
        bytes: &'h [u8],
        start: usize,
        fixed_len: usize,
    ) -> Option<FixedLenFindAll<'v, 'h>> {
        let bp = self.bp_table_for(bytes)?;
        Some(FixedLenFindAll::new(self, bp, bytes, start, fixed_len))
    }

    // -- Bit-parallel execution ------------------------------------------------

    #[inline(always)]
    fn find_raw_bitparallel(
        &self,
        bp: &BitParallelNfa,
        bytes: &[u8],
        start_index: usize,
    ) -> Option<Match> {
        let n = bytes.len();
        let match_mask = 1u64 << bp.match_bit;
        let tbl = &bp.char_transitions;
        let mut best_match: Option<Match> = None;

        // Per-state match origin; valid only when the corresponding bit is set
        // in `current`.  Stale values for inactive bits are never read.
        let mut origins = [usize::MAX; 64];
        let mut current: u64 = 0;
        let mut pos = start_index;

        let mut cache_guard = self.dfa_cache.lock().unwrap();
        let cache = &mut *cache_guard;
        let ck = &mut cache.keys;
        let cb = &mut cache.byte;
        let cv = &mut cache.val;

        if self.start_filter.has_filter() {
            match self.start_filter.find_next_from(bytes, pos) {
                Some(p) => pos = p,
                None => return None,
            }
        }

        while pos <= n {
            // 1. Spawn new threads at `pos` (gated by leftmost-match bound).
            let spawn_ok = best_match.as_ref().is_none_or(|m| pos <= m.start);
            if spawn_ok {
                let new_bits = bp.initial & !current;
                if new_bits != 0 {
                    current |= bp.initial;
                    let mut b = new_bits;
                    while b != 0 {
                        let i = b.trailing_zeros() as usize;
                        b &= b - 1;
                        origins[i] = pos;
                    }
                }
            }

            // 2. Check accepting state.
            if current & match_mask != 0 {
                let m = Match {
                    start: origins[bp.match_bit],
                    end: pos,
                };
                let replace = best_match
                    .as_ref()
                    .is_none_or(|e| m.start < e.start || (m.start == e.start && m.end > e.end));
                if replace {
                    best_match = Some(m);
                }
            }

            if pos == n {
                break;
            }

            let byte = bytes[pos];

            // 3. Next bitmask: try lazy DFA cache first, then flat table scan.
            let slot = (current
                .wrapping_mul(0x9e3779b97f4a7c15)
                .wrapping_add((byte as u64).wrapping_mul(0x517cc1b727220a95))
                >> 54) as usize
                & (DFA_CACHE_SIZE - 1);
            let next = if ck[slot] == current && cb[slot] == byte {
                cv[slot]
            } else {
                let mut nxt = 0u64;
                let mut active = current;
                while active != 0 {
                    let i = active.trailing_zeros() as usize;
                    active &= active - 1;
                    // SAFETY: i < n_states <= 64; byte is a valid u8 index (0-255).
                    nxt |= unsafe { *tbl.get_unchecked(i * 256 + byte as usize) };
                }
                ck[slot] = current;
                cb[slot] = byte;
                cv[slot] = nxt;
                nxt
            };

            {
                let old_origins = origins;

                // a. Reset stale values for bits that are newly active (0->1).
                let newly = next & !current;
                let mut nb = newly;
                while nb != 0 {
                    let j = nb.trailing_zeros() as usize;
                    nb &= nb - 1;
                    origins[j] = usize::MAX;
                }
                let persisting = next & current;
                let mut p = persisting;
                while p != 0 {
                    let j = p.trailing_zeros() as usize;
                    p &= p - 1;
                    let self_loop =
                        unsafe { *tbl.get_unchecked(j * 256 + byte as usize) } & (1u64 << j) != 0;
                    if !self_loop {
                        origins[j] = usize::MAX;
                    }
                }
                let mut active = current;
                while active != 0 {
                    let i = active.trailing_zeros() as usize;
                    active &= active - 1;
                    let tgt = unsafe { *tbl.get_unchecked(i * 256 + byte as usize) };
                    let tgt_in_next = tgt & next;
                    if tgt_in_next != 0 {
                        let orig_i = old_origins[i];
                        let mut t = tgt_in_next;
                        while t != 0 {
                            let j = t.trailing_zeros() as usize;
                            t &= t - 1;
                            if orig_i < origins[j] {
                                origins[j] = orig_i;
                            }
                        }
                    }
                }
            }

            current = next;

            if best_match.is_some() && current == 0 {
                break;
            }

            pos += 1;

            // 5. Jump ahead when threads die and the prefilter can help.
            if current == 0 && self.start_filter.has_filter() {
                match self.start_filter.find_next_from(bytes, pos) {
                    Some(p) => pos = p,
                    None => break,
                }
            }
        }

        best_match
    }

    // -- Epsilon closure ------------------------------------------------------
}

pub struct BitParallelFindAll<'v, 'h> {
    vm: &'v PikeVM,
    bp: &'v BitParallelNfa,
    bytes: &'h [u8],
    /// Resume position for the next match search (`m.end.max(m.start + 1)`
    /// of the last yielded match, or the initial `start`).
    pos: usize,
    origins: [usize; 64],
    exhausted: bool,
}

impl<'v, 'h> BitParallelFindAll<'v, 'h> {
    fn new(vm: &'v PikeVM, bp: &'v BitParallelNfa, bytes: &'h [u8], start: usize) -> Self {
        Self {
            vm,
            bp,
            bytes,
            pos: start,
            origins: [0; 64],
            exhausted: false,
        }
    }
}

impl<'v, 'h> Iterator for BitParallelFindAll<'v, 'h> {
    type Item = Match;

    fn next(&mut self) -> Option<Match> {
        if self.exhausted {
            return None;
        }

        let bp = self.bp;
        let bytes = self.bytes;
        let n = bytes.len();
        let match_mask = 1u64 << bp.match_bit;
        let tbl = &bp.char_transitions;
        let mut best_match: Option<Match> = None;
        let mut current: u64 = 0;
        let mut pos = self.pos;

        let mut cache_guard = self.vm.dfa_cache.lock().unwrap();
        let cache = &mut *cache_guard;
        let ck = &mut cache.keys;
        let cb = &mut cache.byte;
        let cv = &mut cache.val;

        let start_filter = &self.vm.start_filter;
        if start_filter.has_filter() {
            match start_filter.find_next_from(bytes, pos) {
                Some(p) => pos = p,
                None => {
                    self.exhausted = true;
                    return None;
                }
            }
        }

        while pos <= n {
            let spawn_ok = best_match.as_ref().is_none_or(|m| pos <= m.start);
            if spawn_ok {
                let new_bits = bp.initial & !current;
                if new_bits != 0 {
                    current |= bp.initial;
                    let mut b = new_bits;
                    while b != 0 {
                        let i = b.trailing_zeros() as usize;
                        b &= b - 1;
                        self.origins[i] = pos;
                    }
                }
            }

            if current & match_mask != 0 {
                let m = Match {
                    start: self.origins[bp.match_bit],
                    end: pos,
                };
                let replace = best_match
                    .as_ref()
                    .is_none_or(|e| m.start < e.start || (m.start == e.start && m.end > e.end));
                if replace {
                    best_match = Some(m);
                }
            }

            if pos == n {
                break;
            }

            let byte = bytes[pos];

            let slot = (current
                .wrapping_mul(0x9e3779b97f4a7c15)
                .wrapping_add((byte as u64).wrapping_mul(0x517cc1b727220a95))
                >> 54) as usize
                & (DFA_CACHE_SIZE - 1);
            let next = if ck[slot] == current && cb[slot] == byte {
                cv[slot]
            } else {
                let mut nxt = 0u64;
                let mut active = current;
                while active != 0 {
                    let i = active.trailing_zeros() as usize;
                    active &= active - 1;
                    // SAFETY: i < n_states <= 64; byte is a valid u8 index (0-255).
                    nxt |= unsafe { *tbl.get_unchecked(i * 256 + byte as usize) };
                }
                ck[slot] = current;
                cb[slot] = byte;
                cv[slot] = nxt;
                nxt
            };

            // See `find_raw_bitparallel`'s matching block for why this can't
            // be skipped when `next == current` (it used to be, unsoundly).
            {
                let old_origins = self.origins;

                let newly = next & !current;
                let mut nb = newly;
                while nb != 0 {
                    let j = nb.trailing_zeros() as usize;
                    nb &= nb - 1;
                    self.origins[j] = usize::MAX;
                }
                let persisting = next & current;
                let mut p = persisting;
                while p != 0 {
                    let j = p.trailing_zeros() as usize;
                    p &= p - 1;
                    let self_loop =
                        unsafe { *tbl.get_unchecked(j * 256 + byte as usize) } & (1u64 << j) != 0;
                    if !self_loop {
                        self.origins[j] = usize::MAX;
                    }
                }
                let mut active = current;
                while active != 0 {
                    let i = active.trailing_zeros() as usize;
                    active &= active - 1;
                    let tgt = unsafe { *tbl.get_unchecked(i * 256 + byte as usize) };
                    let tgt_in_next = tgt & next;
                    if tgt_in_next != 0 {
                        let orig_i = old_origins[i];
                        let mut t = tgt_in_next;
                        while t != 0 {
                            let j = t.trailing_zeros() as usize;
                            t &= t - 1;
                            if orig_i < self.origins[j] {
                                self.origins[j] = orig_i;
                            }
                        }
                    }
                }
            }

            current = next;

            if best_match.is_some() && current == 0 {
                break;
            }

            pos += 1;

            if current == 0 && start_filter.has_filter() {
                match start_filter.find_next_from(bytes, pos) {
                    Some(p) => pos = p,
                    None => break,
                }
            }
        }

        match best_match {
            Some(m) => {
                self.pos = m.end.max(m.start + 1);
                Some(m)
            }
            None => {
                self.exhausted = true;
                None
            }
        }
    }
}

pub struct FixedLenFindAll<'v, 'h> {
    vm: &'v PikeVM,
    bp: &'v BitParallelNfa,
    bytes: &'h [u8],
    fixed_len: usize,
    pos: usize,
    exhausted: bool,
}

impl<'v, 'h> FixedLenFindAll<'v, 'h> {
    fn new(
        vm: &'v PikeVM,
        bp: &'v BitParallelNfa,
        bytes: &'h [u8],
        start: usize,
        fixed_len: usize,
    ) -> Self {
        Self {
            vm,
            bp,
            bytes,
            fixed_len,
            pos: start,
            exhausted: false,
        }
    }
}

impl<'v, 'h> Iterator for FixedLenFindAll<'v, 'h> {
    type Item = Match;

    fn next(&mut self) -> Option<Match> {
        if self.exhausted {
            return None;
        }

        let bp = self.bp;
        let bytes = self.bytes;
        let n = bytes.len();
        let match_mask = 1u64 << bp.match_bit;
        let tbl = &bp.char_transitions;
        let mut current: u64 = 0;
        let mut pos = self.pos;

        let mut cache_guard = self.vm.dfa_cache.lock().unwrap();
        let cache = &mut *cache_guard;
        let ck = &mut cache.keys;
        let cb = &mut cache.byte;
        let cv = &mut cache.val;

        let start_filter = &self.vm.start_filter;
        if start_filter.has_filter() {
            match start_filter.find_next_from(bytes, pos) {
                Some(p) => pos = p,
                None => {
                    self.exhausted = true;
                    return None;
                }
            }
        }

        while pos <= n {
            current |= bp.initial;

            if current & match_mask != 0 {
                let m = Match {
                    start: pos - self.fixed_len,
                    end: pos,
                };
                self.pos = m.end.max(m.start + 1);
                return Some(m);
            }

            if pos == n {
                break;
            }

            let byte = bytes[pos];

            let slot = (current
                .wrapping_mul(0x9e3779b97f4a7c15)
                .wrapping_add((byte as u64).wrapping_mul(0x517cc1b727220a95))
                >> 54) as usize
                & (DFA_CACHE_SIZE - 1);
            current = if ck[slot] == current && cb[slot] == byte {
                cv[slot]
            } else {
                let mut nxt = 0u64;
                let mut active = current;
                while active != 0 {
                    let i = active.trailing_zeros() as usize;
                    active &= active - 1;
                    // SAFETY: i < n_states <= 64; byte is a valid u8 index (0-255).
                    nxt |= unsafe { *tbl.get_unchecked(i * 256 + byte as usize) };
                }
                ck[slot] = current;
                cb[slot] = byte;
                cv[slot] = nxt;
                nxt
            };

            pos += 1;

            if current == 0 && start_filter.has_filter() {
                match start_filter.find_next_from(bytes, pos) {
                    Some(p) => pos = p,
                    None => break,
                }
            }
        }

        self.exhausted = true;
        None
    }
}

impl PikeVM {
    // -- Epsilon closure -------------------------------------------------------

    /// Add `state_id` (and all epsilon-reachable states) into `list`.
    fn add_epsilon<H: Haystack>(
        nfa: &Nfa,
        list: &mut ThreadList,
        stack: &mut Vec<(usize, usize)>,
        state_id: usize,
        origin: usize,
        pos: usize,
        text: &H,
    ) {
        stack.push((state_id, origin));
        while let Some((sid, orig)) = stack.pop() {
            if list.contains(sid) {
                continue; // already active this round, first origin wins
            }
            list.insert(sid, orig);
            match nfa.states[sid] {
                State::Jump(next) => stack.push((next, orig)),
                State::Split(s1, s2) => {
                    stack.push((s2, orig));
                    stack.push((s1, orig));
                }
                State::Save(_, next) => stack.push((next, orig)),
                State::AnchorStart(next) => {
                    if pos == 0 {
                        stack.push((next, orig));
                    }
                }
                State::AnchorEnd(next) => {
                    if pos == text.len() {
                        stack.push((next, orig));
                    }
                }
                State::WordBoundary(next) => {
                    if is_word_boundary(text, pos) {
                        stack.push((next, orig));
                    }
                }
                State::WordStart(next) => {
                    if is_word_start(text, pos) {
                        stack.push((next, orig));
                    }
                }
                State::WordEnd(next) => {
                    if is_word_end(text, pos) {
                        stack.push((next, orig));
                    }
                }
                _ => {} // consuming state - nothing to follow
            }
        }
    }

    // -- Execution paths -------------------------------------------------------

    #[inline(always)]
    fn find_raw_prefilter<H: Haystack>(
        &self,
        text: H,
        bytes: &[u8],
        start_index: usize,
    ) -> Option<Match> {
        let mut ctx_guard = self.ctx.lock().unwrap();
        let ctx = &mut *ctx_guard;
        ctx.reset();

        let len = text.len();
        let mut best_match: Option<Match> = None;
        let mut pos = start_index;

        while pos <= len {
            // Jump to next potential start when no threads are active.
            if ctx.current.is_empty() && pos < len {
                match self.start_filter.find_next_from(bytes, pos) {
                    Some(p) => pos = p,
                    None => break,
                }
            }

            // Spawn a new thread if this byte could start a match.
            let at_start = pos < len && self.start_filter.matches_byte(bytes[pos]);
            if at_start {
                let spawn_ok = best_match.as_ref().is_none_or(|m| pos <= m.start);
                if spawn_ok {
                    Self::add_epsilon(
                        &self.nfa,
                        &mut ctx.current,
                        &mut ctx.epsilon_stack,
                        self.nfa.start,
                        pos,
                        pos,
                        &text,
                    );
                }
            }

            // Check for accepting state.
            if let Some(match_idx) = ctx.current.position_of(self.nfa.match_state) {
                let origin = ctx.current.seen_origin[self.nfa.match_state];
                best_match = Some(Match {
                    start: origin,
                    end: pos,
                });
                ctx.current.states.truncate(match_idx);
            }

            if pos == len {
                break;
            }

            let (char_val, char_len) = read_char(bytes, pos);
            ctx.next.clear();

            for i in 0..ctx.current.states.len() {
                let pc = ctx.current.states[i];
                let origin = ctx.current.seen_origin[pc];
                if let Some(next_id) = self.get_next_state(pc, char_val) {
                    Self::add_epsilon(
                        &self.nfa,
                        &mut ctx.next,
                        &mut ctx.epsilon_stack,
                        next_id,
                        origin,
                        pos + char_len,
                        &text,
                    );
                }
            }

            std::mem::swap(&mut ctx.current, &mut ctx.next);

            if best_match.is_some() && ctx.current.is_empty() {
                break;
            }

            pos += char_len;
        }

        best_match
    }

    #[inline(always)]
    fn find_raw_baseline<H: Haystack>(
        &self,
        text: H,
        bytes: &[u8],
        start_index: usize,
    ) -> Option<Match> {
        let mut ctx_guard = self.ctx.lock().unwrap();
        let ctx = &mut *ctx_guard;
        ctx.reset();

        let len = text.len();
        let mut best_match: Option<Match> = None;
        let mut pos = start_index;

        while pos <= len {
            let spawn_ok = best_match.as_ref().is_none_or(|m| pos <= m.start);
            if spawn_ok {
                Self::add_epsilon(
                    &self.nfa,
                    &mut ctx.current,
                    &mut ctx.epsilon_stack,
                    self.nfa.start,
                    pos,
                    pos,
                    &text,
                );
            }

            if let Some(match_idx) = ctx.current.position_of(self.nfa.match_state) {
                let origin = ctx.current.seen_origin[self.nfa.match_state];
                best_match = Some(Match {
                    start: origin,
                    end: pos,
                });
                ctx.current.states.truncate(match_idx);
            }

            if pos == len {
                break;
            }

            let (char_val, char_len) = read_char(bytes, pos);
            ctx.next.clear();

            for i in 0..ctx.current.states.len() {
                let pc = ctx.current.states[i];
                let origin = ctx.current.seen_origin[pc];
                if let Some(next_id) = self.get_next_state(pc, char_val) {
                    Self::add_epsilon(
                        &self.nfa,
                        &mut ctx.next,
                        &mut ctx.epsilon_stack,
                        next_id,
                        origin,
                        pos + char_len,
                        &text,
                    );
                }
            }

            std::mem::swap(&mut ctx.current, &mut ctx.next);

            if best_match.is_some() && ctx.current.is_empty() {
                break;
            }

            pos += char_len;
        }

        best_match
    }

    /// Fallback for non-contiguous haystacks: character-by-character NFA simulation.
    fn find_raw_char<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        let mut ctx_guard = self.ctx.lock().unwrap();
        let ctx = &mut *ctx_guard;
        ctx.reset();

        let len = text.len();
        let mut best_match: Option<Match> = None;
        let mut pos = start_index;

        while pos <= len {
            let spawn_ok = best_match.as_ref().is_none_or(|m| pos <= m.start);
            if spawn_ok {
                Self::add_epsilon(
                    &self.nfa,
                    &mut ctx.current,
                    &mut ctx.epsilon_stack,
                    self.nfa.start,
                    pos,
                    pos,
                    &text,
                );
            }

            if let Some(match_idx) = ctx.current.position_of(self.nfa.match_state) {
                let origin = ctx.current.seen_origin[self.nfa.match_state];
                best_match = Some(Match {
                    start: origin,
                    end: pos,
                });
                ctx.current.states.truncate(match_idx);
            }

            if pos == len {
                break;
            }

            let (char_val, char_len) = match text.char_at(pos) {
                Some(c) => c,
                None => break,
            };
            ctx.next.clear();

            for i in 0..ctx.current.states.len() {
                let pc = ctx.current.states[i];
                let origin = ctx.current.seen_origin[pc];
                if let Some(next_id) = self.get_next_state(pc, char_val) {
                    Self::add_epsilon(
                        &self.nfa,
                        &mut ctx.next,
                        &mut ctx.epsilon_stack,
                        next_id,
                        origin,
                        pos + char_len,
                        &text,
                    );
                }
            }

            std::mem::swap(&mut ctx.current, &mut ctx.next);

            if best_match.is_some() && ctx.current.is_empty() {
                break;
            }

            pos += char_len;
        }

        best_match
    }


    pub fn find_captures<H: Haystack>(
        &self,
        text: H,
        start_index: usize,
        num_slots: usize,
    ) -> Option<(Match, Vec<Option<usize>>)> {
        let num_states = self.nfa.states.len();
        let mut current = CaptureThreadList::new(num_states, num_slots);
        let mut next_list = CaptureThreadList::new(num_states, num_slots);
        let mut stack: Vec<(usize, usize, Vec<Option<usize>>)> = Vec::new();

        let len = text.len();
        let mut best: Option<(Match, Vec<Option<usize>>)> = None;
        let mut pos = start_index;

        while pos <= len {
            let spawn_ok = best.as_ref().is_none_or(|(m, _)| pos <= m.start);
            if spawn_ok {
                Self::add_epsilon_captures(
                    &self.nfa,
                    &mut current,
                    &mut stack,
                    self.nfa.start,
                    pos,
                    vec![None; num_slots],
                    pos,
                    &text,
                );
            }

            if let Some(match_idx) = current.position_of(self.nfa.match_state) {
                let thread = &current.data[self.nfa.match_state];
                let new_match = Match {
                    start: thread.origin,
                    end: pos,
                };
                best = Some((new_match, thread.slots.clone()));
                current.states.truncate(match_idx);
            }

            if pos == len {
                break;
            }

            let (char_val, char_len) = match text.char_at(pos) {
                Some(c) => c,
                None => break,
            };
            next_list.clear();

            for i in 0..current.states.len() {
                let pc = current.states[i];
                let origin = current.data[pc].origin;
                let slots = current.data[pc].slots.clone();
                if let Some(next_id) = self.get_next_state(pc, char_val) {
                    Self::add_epsilon_captures(
                        &self.nfa,
                        &mut next_list,
                        &mut stack,
                        next_id,
                        origin,
                        slots,
                        pos + char_len,
                        &text,
                    );
                }
            }

            std::mem::swap(&mut current, &mut next_list);

            if best.is_some() && current.is_empty() {
                break;
            }

            pos += char_len;
        }

        best
    }

    fn add_epsilon_captures<H: Haystack>(
        nfa: &Nfa,
        list: &mut CaptureThreadList,
        stack: &mut Vec<(usize, usize, Vec<Option<usize>>)>,
        state_id: usize,
        origin: usize,
        slots: Vec<Option<usize>>,
        pos: usize,
        text: &H,
    ) {
        stack.push((state_id, origin, slots));
        while let Some((sid, orig, slots)) = stack.pop() {
            if list.contains(sid) {
                continue; // already active this round, first (highest-priority) claim wins
            }
            list.insert(sid, orig, &slots);
            match &nfa.states[sid] {
                State::Jump(next) => stack.push((*next, orig, slots)),
                State::Split(s1, s2) => {
                    // Push s2 first so s1 - the greedy-preferred branch by
                    // construction - pops (and fully explores) first.
                    stack.push((*s2, orig, slots.clone()));
                    stack.push((*s1, orig, slots));
                }
                State::Save(slot, next) => {
                    let mut s = slots;
                    if *slot < s.len() {
                        s[*slot] = Some(pos);
                    }
                    stack.push((*next, orig, s));
                }
                State::AnchorStart(next) => {
                    if pos == 0 {
                        stack.push((*next, orig, slots));
                    }
                }
                State::AnchorEnd(next) => {
                    if pos == text.len() {
                        stack.push((*next, orig, slots));
                    }
                }
                State::WordBoundary(next) => {
                    if is_word_boundary(text, pos) {
                        stack.push((*next, orig, slots));
                    }
                }
                State::WordStart(next) => {
                    if is_word_start(text, pos) {
                        stack.push((*next, orig, slots));
                    }
                }
                State::WordEnd(next) => {
                    if is_word_end(text, pos) {
                        stack.push((*next, orig, slots));
                    }
                }
                _ => {} // consuming state - nothing to follow
            }
        }
    }

    // -- State transitions -----------------------------------------------------

    #[inline(always)]
    fn get_next_state(&self, sid: usize, c: char) -> Option<usize> {
        match &self.nfa.states[sid] {
            State::Char(expect, next) => {
                if *expect == c {
                    Some(*next)
                } else {
                    None
                }
            }
            State::Class(class, next) => {
                if self.match_class(class, c) {
                    Some(*next)
                } else {
                    None
                }
            }
            State::Any(next) => {
                if c != '\n' {
                    Some(*next)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn match_class(&self, class: &CharClass, c: char) -> bool {
        use CharClass::*;
        let ic = self.nfa.flags.ignore_case.unwrap_or(false);
        match class {
            Digit => c.is_ascii_digit(),
            NonDigit => !c.is_ascii_digit(),
            Word => c.is_alphanumeric() || c == '_',
            NonWord => !(c.is_alphanumeric() || c == '_'),
            Whitespace => c.is_whitespace(),
            NonWhitespace => !c.is_whitespace(),
            Dot => self.nfa.flags.dotall || c != '\n',
            Lowercase => c.is_lowercase() || (ic && c.is_uppercase()),
            NonLowercase => !c.is_lowercase() && (!ic || !c.is_uppercase()),
            Uppercase => c.is_uppercase() || (ic && c.is_lowercase()),
            NonUppercase => !c.is_uppercase() && (!ic || !c.is_lowercase()),
            Hex => c.is_ascii_hexdigit(),
            NonHex => !c.is_ascii_hexdigit(),
            Octal => c.is_digit(8),
            NonOctal => !c.is_digit(8),
            Alphanumeric => c.is_alphanumeric(),
            NonAlphanumeric => !c.is_alphanumeric(),
            Punctuation => c.is_ascii_punctuation(),
            NonPunctuation => !c.is_ascii_punctuation(),
            WordStart => c.is_alphabetic() || c == '_',
            NonWordStart => !(c.is_alphabetic() || c == '_'),
            Set { chars, negated } => {
                let found = chars.iter().any(|range| {
                    if c >= range.start && c <= range.end {
                        return true;
                    }
                    if ic {
                        if c.to_lowercase()
                            .any(|lc| lc >= range.start && lc <= range.end)
                        {
                            return true;
                        }
                        if c.to_uppercase()
                            .any(|uc| uc >= range.start && uc <= range.end)
                        {
                            return true;
                        }
                    }
                    false
                });
                if *negated { !found } else { found }
            }
        }
    }
}

// -- Disjoint-greedy-segments execution ----------------------------------------

#[inline]
fn match_segments_from(segs: &[super::Segment], bytes: &[u8], start: usize) -> Option<usize> {
    let n = bytes.len();
    let mut pos = start;
    for seg in segs {
        let mut count = 0usize;
        while count < seg.max && pos + count < n && seg.alphabet.contains(bytes[pos + count]) {
            count += 1;
        }
        if count < seg.min {
            return None;
        }
        pos += count;
    }
    Some(pos)
}

/// Length of the greedy run of `alphabet`-matching bytes starting at `pos`.
#[inline]
fn greedy_run_len(bytes: &[u8], pos: usize, alphabet: &super::SegBits) -> usize {
    let n = bytes.len();
    let mut count = 0usize;
    while pos + count < n && alphabet.contains(bytes[pos + count]) {
        count += 1;
    }
    count
}

fn find_via_segments(segs: &[super::Segment], bytes: &[u8], start_index: usize) -> Option<Match> {
    let n = bytes.len();
    let mut pos = start_index;
    let first = &segs[0];
    while pos <= n {
        if first.min >= 1 {
            match bytes[pos..]
                .iter()
                .position(|&b| first.alphabet.contains(b))
            {
                Some(off) => pos += off,
                None => return None,
            }
        }
        match match_segments_from(segs, bytes, pos) {
            Some(end) => return Some(Match { start: pos, end }),
            None => {
                if first.max == usize::MAX {
                    pos += greedy_run_len(bytes, pos, &first.alphabet).max(1);
                } else {
                    pos += 1;
                }
            }
        }
    }
    None
}

/// Streaming `find_all` iterator for the disjoint-greedy-segments fast path.
pub struct SegmentsFindAll<'h, 's> {
    segs: &'s [super::Segment],
    bytes: &'h [u8],
    pos: usize,
    exhausted: bool,
}

impl<'h, 's> SegmentsFindAll<'h, 's> {
    fn new(segs: &'s [super::Segment], bytes: &'h [u8], start: usize) -> Self {
        Self {
            segs,
            bytes,
            pos: start,
            exhausted: false,
        }
    }
}

impl<'h, 's> Iterator for SegmentsFindAll<'h, 's> {
    type Item = Match;

    fn next(&mut self) -> Option<Match> {
        if self.exhausted {
            return None;
        }
        match find_via_segments(self.segs, self.bytes, self.pos) {
            Some(m) => {
                self.pos = m.end.max(m.start + 1);
                Some(m)
            }
            None => {
                self.exhausted = true;
                None
            }
        }
    }
}

// -- Helpers -------------------------------------------------------------------

/// True if `class` only ever matches ASCII characters. The bit-parallel table is
/// byte-keyed, so it can only represent classes that never match a multibyte char.
pub(crate) fn class_is_ascii_only(class: &CharClass) -> bool {
    use CharClass::*;
    match class {
        Digit | Hex | Octal | Punctuation => true,
        Set {
            chars,
            negated: false,
        } => chars.iter().all(|r| r.end.is_ascii()),
        _ => false,
    }
}

/// Static class check used during bit-parallel table precomputation.
pub(crate) fn matches_class_static(class: &CharClass, c: char, ic: bool, dotall: bool) -> bool {
    use CharClass::*;
    match class {
        Digit => c.is_ascii_digit(),
        NonDigit => !c.is_ascii_digit(),
        Word => c.is_alphanumeric() || c == '_',
        NonWord => !(c.is_alphanumeric() || c == '_'),
        Whitespace => c.is_whitespace(),
        NonWhitespace => !c.is_whitespace(),
        Dot => dotall || c != '\n',
        Lowercase => c.is_lowercase() || (ic && c.is_uppercase()),
        NonLowercase => !c.is_lowercase() && (!ic || !c.is_uppercase()),
        Uppercase => c.is_uppercase() || (ic && c.is_lowercase()),
        NonUppercase => !c.is_uppercase() && (!ic || !c.is_lowercase()),
        Hex => c.is_ascii_hexdigit(),
        NonHex => !c.is_ascii_hexdigit(),
        Octal => c.is_digit(8),
        NonOctal => !c.is_digit(8),
        Alphanumeric => c.is_alphanumeric(),
        NonAlphanumeric => !c.is_alphanumeric(),
        Punctuation => c.is_ascii_punctuation(),
        NonPunctuation => !c.is_ascii_punctuation(),
        WordStart => c.is_alphabetic() || c == '_',
        NonWordStart => !(c.is_alphabetic() || c == '_'),
        Set { chars, negated } => {
            let found = chars.iter().any(|range| {
                if c >= range.start && c <= range.end {
                    return true;
                }
                if ic {
                    if c.to_lowercase()
                        .any(|lc| lc >= range.start && lc <= range.end)
                    {
                        return true;
                    }
                    if c.to_uppercase()
                        .any(|uc| uc >= range.start && uc <= range.end)
                    {
                        return true;
                    }
                }
                false
            });
            if *negated { !found } else { found }
        }
    }
}

#[inline(always)]
fn read_char(bytes: &[u8], pos: usize) -> (char, usize) {
    let b = bytes[pos];
    if b < 0x80 {
        (b as char, 1)
    } else {
        // SAFETY: pos is a valid UTF-8 char boundary within a &str-backed slice.
        let s = unsafe { std::str::from_utf8_unchecked(&bytes[pos..]) };
        let c = s.chars().next().unwrap_or('\0');
        (c, c.len_utf8())
    }
}

fn is_word_boundary<H: Haystack>(text: &H, pos: usize) -> bool {
    is_word_char(text.char_before(pos)) != is_word_char(text.char_at(pos).map(|(c, _)| c))
}

fn is_word_start<H: Haystack>(text: &H, pos: usize) -> bool {
    !is_word_char(text.char_before(pos)) && is_word_char(text.char_at(pos).map(|(c, _)| c))
}

fn is_word_end<H: Haystack>(text: &H, pos: usize) -> bool {
    is_word_char(text.char_before(pos)) && !is_word_char(text.char_at(pos).map(|(c, _)| c))
}

fn is_word_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}
