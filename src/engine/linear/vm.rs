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
        if self.bytes.is_empty() {
            return Some(start);
        }
        if haystack.len().saturating_sub(start) <= CI_SMALL_THRESHOLD {
            return naive_ci_find(haystack, &self.bytes, start);
        }
        let mut buf: Vec<u8> = Vec::with_capacity(CI_CHUNK + self.bytes.len());
        ci_chunked_search(&self.finder, self.bytes.len(), haystack, start, &mut buf).map(
            |(abs, _rel, _core_len, _chunk_base)| abs,
        )
    }
}

const CI_CHUNK: usize = 16384;

const CI_SMALL_THRESHOLD: usize = 4096;

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

#[allow(clippy::large_enum_variant)]
pub enum LiteralFindIter<'h, 'n> {
    CaseSensitive {
        /// Streams through `haystack[start..]` yielding match offsets relative to `offset`.
        inner: memchr::memmem::FindIter<'h, 'n>,
        /// Absolute start position so returned Match positions are haystack-relative.
        offset: usize,
        lit_len: usize,
    },
    /// Below `CI_SMALL_THRESHOLD` total remaining bytes: same zero-copy scan
    /// as `naive_ci_find`, avoiding the chunked path's fixed allocation cost.
    CaseInsensitiveSmall {
        haystack: &'h [u8],
        pos: usize,
        lit: &'n Literal,
    },
    CaseInsensitive {
        haystack: &'h [u8],
        lit: &'n Literal,
        /// Lowercased window into `haystack[chunk_base..]`, reused across chunks
        /// so `find_all` pays the lowercasing cost once per `CI_CHUNK` bytes
        /// rather than once per match.
        buf: Vec<u8>,
        /// Absolute offset in `haystack` that `buf[0]` corresponds to.
        chunk_base: usize,
        /// Number of bytes at the front of `buf` that are valid match-start
        /// positions (the rest is trailing overlap for boundary matches).
        core_len: usize,
        /// Next offset within `buf` to resume the `memmem` search from.
        search_from: usize,
        exhausted: bool,
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
        } else if lit.bytes.is_empty() || haystack.len().saturating_sub(start) <= CI_SMALL_THRESHOLD
        {
            LiteralFindIter::CaseInsensitiveSmall {
                haystack,
                pos: start,
                lit,
            }
        } else {
            LiteralFindIter::CaseInsensitive {
                haystack,
                lit,
                buf: Vec::new(),
                chunk_base: start,
                core_len: 0,
                search_from: 0,
                exhausted: lit.bytes.is_empty(),
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
            LiteralFindIter::CaseInsensitiveSmall { haystack, pos, lit } => {
                if lit.bytes.is_empty() {
                    return None;
                }
                let start = naive_ci_find(haystack, &lit.bytes, *pos)?;
                let end = start + lit.len();
                *pos = end;
                Some(Match { start, end })
            }
            LiteralFindIter::CaseInsensitive {
                haystack,
                lit,
                buf,
                chunk_base,
                core_len,
                search_from,
                exhausted,
            } => {
                if *exhausted {
                    return None;
                }
                loop {
                    if let Some(rel) = lit.finder.find(&buf[*search_from..]) {
                        let abs_rel = *search_from + rel;
                        if abs_rel < *core_len {
                            let start = *chunk_base + abs_rel;
                            *search_from = abs_rel + lit.len();
                            return Some(Match {
                                start,
                                end: start + lit.len(),
                            });
                        }
                    }
                    let next_base = *chunk_base + *core_len;
                    if next_base >= haystack.len() {
                        *exhausted = true;
                        return None;
                    }
                    *chunk_base = next_base;
                    let core_end = (*chunk_base + CI_CHUNK).min(haystack.len());
                    let ext_end = (core_end + lit.len() - 1).min(haystack.len());
                    buf.clear();
                    buf.extend(
                        haystack[*chunk_base..ext_end]
                            .iter()
                            .map(u8::to_ascii_lowercase),
                    );
                    *core_len = core_end - *chunk_base;
                    *search_from = 0;
                }
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

    /// O(1) origin lookup.
    #[inline(always)]
    fn get_origin(&self, pc: usize) -> Option<usize> {
        if self.seen_gen[pc] == self.generation {
            Some(self.seen_origin[pc])
        } else {
            None
        }
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.states.is_empty()
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
    /// Flat n_states x 256 transition table (row-major, stride = 256).
    /// Entry `char_transitions[state * 256 + byte]` holds the bitmask of NFA states
    /// reachable by consuming `byte` from `state`, including epsilon closure.
    /// Stored as one contiguous allocation for L1/L2 cache locality.
    char_transitions: Box<[u64]>,
    /// Epsilon closure of the start state (the initial DFA state).
    initial: u64,
    /// Bit position of the NFA match state.
    match_bit: usize,
}

impl BitParallelNfa {
    /// Try to build a precomputed table from `nfa`. Returns `None` if the NFA
    /// has > 64 states or contains zero-width assertions (anchor / word boundary).
    ///
    /// Unconditionally safe for any input (ASCII or not): classes that can
    /// match non-ASCII characters (e.g. `\w`, which is Unicode-aware by
    /// default in this engine) and `.`/`Any` are excluded, since the table is
    /// byte-keyed and cannot represent multibyte UTF-8 matching.
    pub fn build(nfa: &Nfa) -> Option<Self> {
        Self::build_inner(nfa, false)
    }

    /// Like `build`, but also accepts `Any` and classes that are only
    /// "not ASCII-only" because of their multibyte-Unicode behavior (`\w`,
    /// `\s`, ...). The resulting table is only correct when the haystack it
    /// is run against is verified all-ASCII first (see `find_raw`): for pure
    /// ASCII input, Unicode-aware classification of a byte in `0..128` is
    /// identical to ASCII classification, so the byte-keyed table (already
    /// only ever populated for `0..=127`, see below) stays exact. Still
    /// excludes zero-width assertions (position-dependent, unrelated to byte
    /// encoding) and literal non-ASCII `Char` states (which simply can never
    /// fire against ASCII-only input - a dead table entry is correct there).
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
                // The table is keyed by single bytes (0..=255) and only populated for
                // ASCII, so it cannot represent matching of multibyte UTF-8 chars.
                // Bail out for any consuming state that can match a non-ASCII char;
                // the char-based Pike VM (`read_char`) handles those correctly.
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

/// Persistent 1 K-entry direct-mapped cache for `(bitmask, byte) -> next_bitmask`.
/// Stored inside PikeVM so it is initialised ONCE and reused across all
/// `find_from` / `find_all` calls.  Entries from a previous call are simply
/// overwritten on collision - no invalidation needed because the mapping is
/// purely a function of the NFA transition table (position-independent).
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

use std::cell::UnsafeCell;

pub struct PikeVM {
    nfa: Nfa,
    start_filter: StartFilter,
    /// If Some, the entire pattern is a literal - skip NFA simulation.
    literal: Option<Literal>,
    /// Bit-parallel NFA for fast simulation (None if NFA too large or has assertions).
    bp_nfa: Option<BitParallelNfa>,
    /// Bit-parallel NFA valid only when the haystack is verified all-ASCII
    /// (see `find_raw`). Populated when `bp_nfa` was disqualified solely by a
    /// class/`.` that can match non-ASCII in general (e.g. `\w`), letting
    /// patterns built from those still take the fast path on ASCII input.
    bp_nfa_ascii: Option<BitParallelNfa>,
    /// UnsafeCell for zero-overhead interior mutability.
    /// SAFETY: PikeVM is used single-threaded per find operation.
    ctx: UnsafeCell<VMContext>,
    /// Persistent DFA cache: initialised once, reused across all find_from calls.
    /// SAFETY: same single-threaded guarantee as `ctx`.
    dfa_cache: UnsafeCell<LazyDfaCache>,
    /// Memoised `is_ascii()` verdict for `bp_nfa_ascii`, keyed by the exact
    /// `(ptr, len)` of the last haystack checked.
    ///
    /// `find_all` calls `find_raw` once per match against the *same*
    /// underlying buffer (only `start_index` advances), so without this,
    /// checking `bytes.is_ascii()` fresh on every call turns an O(n) scan
    /// into O(n * matches) - the check re-scans from byte 0 every time,
    /// dominating the very fast path it exists to enable.
    ascii_cache: std::cell::Cell<(usize, usize, bool)>,
    /// Set when every match of this pattern consumes exactly the same number
    /// of bytes (see `fixed_length` in `mod.rs`). Licenses `find_raw_fixed_len`,
    /// which skips per-position origin tracking entirely: with fixed length,
    /// `start = end - fixed_len` holds for every thread that can ever reach
    /// the accept state, so there is no leftmost-vs-longest ambiguity to
    /// resolve by tracking *which* thread got there.
    fixed_len: Option<usize>,
    /// Set when the pattern is a flat concatenation of quantified
    /// single-atom segments with provably unambiguous greedy consumption
    /// (see `disjoint_greedy_segments` in `mod.rs`). Licenses
    /// `find_via_segments`, which needs no NFA simulation at all - just a
    /// per-segment greedy byte-class scan. Like `bp_nfa_ascii`, only valid
    /// when the haystack is verified all-ASCII first.
    segments: Option<Vec<super::Segment>>,
}

// SAFETY: find operations are inherently single-threaded.
unsafe impl Sync for PikeVM {}

impl PikeVM {
    pub fn new(
        nfa: Nfa,
        start_filter: StartFilter,
        literal: Option<Literal>,
        fixed_len: Option<usize>,
        segments: Option<Vec<super::Segment>>,
    ) -> Self {
        let num_states = nfa.states.len();
        let bp_nfa = if literal.is_none() {
            BitParallelNfa::build(&nfa)
        } else {
            None // literal path bypasses NFA entirely
        };
        // Only worth building the ASCII-gated table when the strict one
        // failed and there's no literal fast path already covering it.
        let bp_nfa_ascii = if literal.is_none() && bp_nfa.is_none() {
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
            ctx: UnsafeCell::new(VMContext::new(num_states)),
            dfa_cache: UnsafeCell::new(LazyDfaCache::new()),
            ascii_cache: std::cell::Cell::new((0, 0, false)),
            fixed_len,
            segments,
        }
    }

    /// `bytes.is_ascii()`, memoised by buffer identity (see `ascii_cache`).
    #[inline]
    fn is_ascii_cached(&self, bytes: &[u8]) -> bool {
        let key = (bytes.as_ptr() as usize, bytes.len());
        let (ptr, len, val) = self.ascii_cache.get();
        if (ptr, len) == key {
            return val;
        }
        let val = bytes.is_ascii();
        self.ascii_cache.set((key.0, key.1, val));
        val
    }

    pub fn find_from<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        self.find_raw(text, start_index)
    }

    /// The bit-parallel table applicable to `bytes` right now, if any: the
    /// unconditionally-safe one, or the ASCII-gated one when `bytes` is
    /// verified all-ASCII. Used to pick a streaming `find_all` strategy once
    /// per call instead of re-deciding (and re-checking `is_ascii`) per match.
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

    /// Streaming find-all entry point for the bit-parallel fast path (see
    /// `BitParallelFindAll`). Returns `None` if no bit-parallel table applies
    /// to `bytes` right now - callers fall back to the general per-match path.
    pub fn find_all_bitparallel<'v, 'h>(
        &'v self,
        bytes: &'h [u8],
        start: usize,
    ) -> Option<BitParallelFindAll<'v, 'h>> {
        let bp = self.bp_table_for(bytes)?;
        Some(BitParallelFindAll::new(self, bp, bytes, start))
    }

    /// Streaming find-all entry point for the disjoint-greedy-segments fast
    /// path (see `find_via_segments`). Returns `None` if it doesn't apply
    /// (no segments decomposition, or `bytes` isn't all-ASCII).
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

    /// The disjoint-greedy-segments decomposition, if this pattern qualifies
    /// (see `disjoint_greedy_segments`). Callers must verify the haystack is
    /// all-ASCII before using it - same contract as `bp_nfa_ascii`.
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
        if let Some(bytes) = text.as_bytes_opt() {
            // Contiguous bytes: use fast paths in priority order.
            if let Some(lit) = &self.literal {
                let pos = lit.find_in(bytes, start_index)?;
                return Some(Match {
                    start: pos,
                    end: pos + lit.len(),
                });
            }
            // Fixed-length fast path: no origin tracking needed at all (see
            // `fixed_len` and `find_raw_fixed_len`). Checked before the
            // general bit-parallel path since it's strictly cheaper whenever
            // it applies.
            if let Some(fixed_len) = self.fixed_len
                && let Some(bp) = self.bp_table_for(bytes)
            {
                return self.find_raw_fixed_len(bp, bytes, start_index, fixed_len);
            }
            // Disjoint-greedy-segments fast path: no NFA/bitmask simulation
            // at all, just a per-segment greedy byte-class scan (see
            // `find_via_segments`). Broader than the fixed-length path above
            // (covers variable-length patterns like `[AC]+G+[TA]+`) but
            // still only reached when fixed-length didn't already apply.
            if let Some(segs) = &self.segments
                && self.is_ascii_cached(bytes)
            {
                return find_via_segments(segs, bytes, start_index);
            }
            // Bit-parallel NFA: replaces add_epsilon stack with table lookups.
            if let Some(bp) = &self.bp_nfa {
                return self.find_raw_bitparallel(bp, bytes, start_index);
            }
            // ASCII-gated bit-parallel NFA: same fast path for patterns using
            // Unicode-aware classes (`\w`, `\s`, ...), valid whenever the
            // haystack turns out to be pure ASCII. `is_ascii()` is a single
            // SIMD-accelerated scan, far cheaper than the generic per-char
            // PikeVM it lets us skip.
            if let Some(bp) = &self.bp_nfa_ascii
                && self.is_ascii_cached(bytes)
            {
                return self.find_raw_bitparallel(bp, bytes, start_index);
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

    /// Bit-parallel scan for a pattern whose every match is exactly
    /// `fixed_len` bytes long (see `fixed_length` in `mod.rs`). No per-position
    /// origin tracking: since *every* thread that can ever reach the accept
    /// state has consumed exactly `fixed_len` bytes to get there, the first
    /// position at which the accept bit turns on is, by construction, both
    /// the leftmost match *and* unambiguous about where it started
    /// (`start = end - fixed_len`) - there is no "which thread got there"
    /// question to resolve, so this returns on the very first hit.
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

        // SAFETY: single-threaded per find operation (see `dfa_cache`).
        let cache = unsafe { &mut *self.dfa_cache.get() };
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

    /// Fast NFA simulation using a precomputed flat 256-column transition table.
    ///
    /// Three key optimisations:
    ///
    /// 1. **Flat contiguous table** - `char_transitions` is one Box<[u64]> with
    ///    stride 256 so that all rows fit in L1/L2 cache without pointer chasing.
    ///
    /// 2. **Stable-bitmask shortcut** - when `next == current` (no state gained
    ///    or lost), origins are already at their historical minimum and the entire
    ///    O(active) origin-update loop is skipped.  Loops like `[AC]+` and `\w+`
    ///    keep a stable bitmask for runs of matching bytes, collapsing per-byte
    ///    cost to a table lookup + comparison.
    ///
    /// 3. **Lazy DFA cache** - 1 K direct-mapped slots cache `(bitmask, byte) ->
    ///    next_bitmask`.  On repeated `(current, byte)` pairs the O(active) loop
    ///    is also skipped for the transition itself.
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

        // Persistent DFA cache: borrow the PikeVM-level cache (initialised once,
        // never re-zeroed between calls) via the existing UnsafeCell.
        // SAFETY: single-threaded per find operation (see struct comment).
        let cache = unsafe { &mut *self.dfa_cache.get() };
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

            // 4. Update origins - skipped on stable bitmask (the common case for
            //    loop bodies like [AC]+ or \w+).  When next == current no state
            //    was gained or lost, so all origins are already at their minimum.
            if next != current {
                // Snapshot pre-transition origins before mutating anything.
                // Without this, a bit that is BOTH a target written earlier
                // in the loop below (because a lower-numbered source feeds
                // it) AND a source read later in the same loop (when its own
                // turn as `i` comes up) would read its own just-written
                // *post*-transition value instead of the pre-transition one
                // it must contribute as a source - silently corrupting
                // whichever downstream target it feeds, depending on bit
                // iteration order.
                let old_origins = origins;

                // a. Reset stale values for bits that are newly active (0->1).
                let newly = next & !current;
                let mut nb = newly;
                while nb != 0 {
                    let j = nb.trailing_zeros() as usize;
                    nb &= nb - 1;
                    origins[j] = usize::MAX;
                }
                // a2. A bit can also persist (stay set from `current` into
                //     `next`) WITHOUT looping back to itself this byte - e.g.
                //     a dead-end accepting state that a *different* source
                //     re-targets fresh this round. Its old origin describes an
                //     occupant that did not actually survive this step, so it
                //     must not be allowed to win the min-compare in (b) against
                //     a genuinely later candidate. Reset it unless state `j`
                //     itself transitions back into `j` (a real self-loop,
                //     where carrying the old origin forward is correct and
                //     intended - see the `(ab)+` case below). Safe to reset
                //     `origins[j]` here (rather than `old_origins[j]`) since
                //     (b) below only ever reads from the untouched snapshot.
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
                // b. Propagate min origin from every source to every target in
                //    `next`.  Must cover ALL targets (not only newly-active ones)
                //    because back-edges (e.g. the loop in `(ab)+`) can deliver a
                //    smaller origin to an already-active state. Sources are read
                //    from `old_origins` (frozen pre-transition), never from
                //    `origins` (being written here), so iteration order over
                //    `current`'s bits can't affect the result.
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

/// Streaming `find_all` iterator for the bit-parallel fast path.
///
/// `find_all` needs one match at a time, but the general-purpose path gets
/// there by calling the single-match matcher fresh for every match found -
/// each call re-derives which fast path applies, re-checks the start filter,
/// and (before this type existed) re-zeroed a 64-entry origins array. For
/// patterns with dense matches (e.g. a match every few bytes) that fixed
/// per-call cost, paid thousands of times, dominated the actual per-byte
/// scanning work it was wrapping.
///
/// This iterator instead runs the exact same leftmost-longest thread
/// simulation as `find_raw_bitparallel`, but keeps `origins` as iterator
/// state reused across matches instead of a fresh array per match. This is
/// sound because `origins[i]` is only ever read while bit `i` is active in
/// `current`/`next` *within the search for the current match* - and any such
/// bit was necessarily assigned a fresh value earlier in that same search
/// (via the initial spawn step or the newly-active transfer step below), so
/// leftover values from a previous match's search are never observed.
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

        // SAFETY: single-threaded per find operation (see `PikeVM::dfa_cache`).
        let cache = unsafe { &mut *self.vm.dfa_cache.get() };
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

            if next != current {
                // See the matching comment in `find_raw_bitparallel`: snapshot
                // pre-transition origins before mutating anything, so a bit
                // written as a target earlier in the loop below (by a
                // lower-numbered source) can't corrupt its own value when it
                // is later read as a source (when its turn as `i` comes up).
                let old_origins = self.origins;

                let newly = next & !current;
                let mut nb = newly;
                while nb != 0 {
                    let j = nb.trailing_zeros() as usize;
                    nb &= nb - 1;
                    self.origins[j] = usize::MAX;
                }
                // A bit that persists into `next` without a genuine self-loop
                // this byte must not have its stale origin win the
                // min-compare below against a later, genuinely-fresh
                // candidate that reaches this state index via another source.
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

/// Streaming `find_all` iterator for the fixed-length fast path (see
/// `PikeVM::find_raw_fixed_len`). No origin array at all - fixed length means
/// `start = end - fixed_len` unconditionally, so there is nothing to track
/// across bytes beyond the current bitmask itself.
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

        // SAFETY: single-threaded per find operation (see `PikeVM::dfa_cache`).
        let cache = unsafe { &mut *self.vm.dfa_cache.get() };
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
                    stack.push((s1, orig));
                    stack.push((s2, orig));
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
        // SAFETY: single-threaded find operation
        let ctx = unsafe { &mut *self.ctx.get() };
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
            if let Some(origin) = ctx.current.get_origin(self.nfa.match_state) {
                let new_match = Match {
                    start: origin,
                    end: pos,
                };
                let replace = match &best_match {
                    None => true,
                    Some(e) => {
                        new_match.start < e.start
                            || (new_match.start == e.start && new_match.end > e.end)
                    }
                };
                if replace {
                    best_match = Some(new_match);
                }
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
        // SAFETY: single-threaded find operation
        let ctx = unsafe { &mut *self.ctx.get() };
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

            if let Some(origin) = ctx.current.get_origin(self.nfa.match_state) {
                let new_match = Match {
                    start: origin,
                    end: pos,
                };
                let replace = match &best_match {
                    None => true,
                    Some(e) => {
                        new_match.start < e.start
                            || (new_match.start == e.start && new_match.end > e.end)
                    }
                };
                if replace {
                    best_match = Some(new_match);
                }
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
        // SAFETY: single-threaded find operation
        let ctx = unsafe { &mut *self.ctx.get() };
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

            if let Some(origin) = ctx.current.get_origin(self.nfa.match_state) {
                let new_match = Match {
                    start: origin,
                    end: pos,
                };
                let replace = match &best_match {
                    None => true,
                    Some(e) => {
                        new_match.start < e.start
                            || (new_match.start == e.start && new_match.end > e.end)
                    }
                };
                if replace {
                    best_match = Some(new_match);
                }
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

/// Match `segs` starting exactly at `start`, consuming each segment greedily
/// (up to its `max`), requiring at least `min`. Under the disjointness
/// invariant `disjoint_greedy_segments` verified at compile time, this is the
/// *only* possible decomposition: if it fails, no decomposition starting at
/// `start` exists at all, so there is nothing to backtrack into.
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

/// Find the leftmost match of `segs` starting at or after `start_index`.
///
/// Scans candidate starts left to right, verifying each via
/// `match_segments_from`. On failure, if the first segment is unbounded
/// (`max == usize::MAX`), every start within the current run of its alphabet
/// fails identically - greedy consumption always eats the whole run
/// regardless of where within it you start, so they all hit the exact same
/// downstream wall - so the whole run is skipped in one step rather than
/// retried byte by byte.
fn find_via_segments(segs: &[super::Segment], bytes: &[u8], start_index: usize) -> Option<Match> {
    let n = bytes.len();
    let mut pos = start_index;
    let first = &segs[0];
    while pos <= n {
        if first.min >= 1 {
            match bytes[pos..].iter().position(|&b| first.alphabet.contains(b)) {
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
fn class_is_ascii_only(class: &CharClass) -> bool {
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

/// Decode one character from `bytes[pos]`. ASCII fast path avoids UTF-8 decode.
///
/// SAFETY invariant (upheld by callers): `pos` is always a char boundary.
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
