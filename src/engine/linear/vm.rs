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
    /// Arbitrary ASCII class stored as a 128-bit membership table.
    /// `mask[b >> 6] & (1 << (b & 63))` is set iff byte `b` can start a match.
    Table128([u64; 2]),
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
            StartFilter::Table128(mask) => sub
                .iter()
                .position(|&b| mask[(b >> 6) as usize] & (1u64 << (b & 63)) != 0)?,
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
            StartFilter::Table128(mask) => mask[(b >> 6) as usize] & (1u64 << (b & 63)) != 0,
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
    /// Prebuilt memmem Finder (owned, 'static) so we never pay the build cost
    /// more than once.  Used only for case-sensitive search; CI falls back to
    /// the memchr2 + verify loop.
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

    /// Find the next occurrence of this literal in `haystack[start..]`.
    /// Returns the absolute byte offset, or None.
    pub fn find_in(&self, haystack: &[u8], start: usize) -> Option<usize> {
        if start >= haystack.len() {
            return None;
        }
        if !self.case_insensitive {
            // Case-sensitive: reuse the prebuilt SIMD Finder.
            self.finder.find(&haystack[start..]).map(|i| i + start)
        } else {
            // Case-insensitive ASCII: memchr2 on first byte then verify rest.
            if self.bytes.is_empty() {
                return Some(start);
            }
            let first = self.bytes[0];
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
                let end = abs + self.bytes.len();
                if end > haystack.len() {
                    return None;
                }
                if haystack[abs..end]
                    .iter()
                    .zip(self.bytes.iter())
                    .all(|(&tb, &pb)| tb.to_ascii_lowercase() == pb)
                {
                    return Some(abs);
                }
                pos = abs + 1;
            }
        }
    }
}

// -- Literal find-all iterator ------------------------------------------------

/// Single-pass iterator over all non-overlapping occurrences of a literal.
///
/// CS path: wraps `memmem::FindIter` - a streaming SIMD searcher that processes
/// the haystack in one pass, yielding one position per match without any per-call
/// SIMD prologue overhead.  The Finder is pre-built in `Literal::new` and borrowed
/// here; no allocation or build cost on `find_all`.
///
/// CI path: memchr2 on first byte + byte-by-byte verify (same as before).
// The case-sensitive variant embeds a memmem `FindIter` (larger than the CI
// variant), but this iterator is short-lived and never stored in bulk, so the
// size difference is not worth an extra heap indirection.
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
            LiteralFindIter::CaseInsensitive { haystack, pos, lit } => {
                let start = lit.find_in(haystack, *pos)?;
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
    pub fn build(nfa: &Nfa) -> Option<Self> {
        let n = nfa.states.len();
        if n > 64 {
            return None;
        }

        // Zero-width assertions make the epsilon closure position-dependent.
        for s in &nfa.states {
            match s {
                State::AnchorStart(_)
                | State::AnchorEnd(_)
                | State::WordBoundary(_)
                | State::WordStart(_)
                | State::WordEnd(_) => return None,
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
    /// UnsafeCell for zero-overhead interior mutability.
    /// SAFETY: PikeVM is used single-threaded per find operation.
    ctx: UnsafeCell<VMContext>,
    /// Persistent DFA cache: initialised once, reused across all find_from calls.
    /// SAFETY: same single-threaded guarantee as `ctx`.
    dfa_cache: UnsafeCell<LazyDfaCache>,
}

// SAFETY: find operations are inherently single-threaded.
unsafe impl Sync for PikeVM {}

impl PikeVM {
    pub fn new(nfa: Nfa, start_filter: StartFilter, literal: Option<Literal>) -> Self {
        let num_states = nfa.states.len();
        let bp_nfa = if literal.is_none() {
            BitParallelNfa::build(&nfa)
        } else {
            None // literal path bypasses NFA entirely
        };
        Self {
            nfa,
            start_filter,
            literal,
            bp_nfa,
            ctx: UnsafeCell::new(VMContext::new(num_states)),
            dfa_cache: UnsafeCell::new(LazyDfaCache::new()),
        }
    }

    pub fn find_from<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        self.find_raw(text, start_index)
    }

    /// Access the precomputed literal, if any.
    #[inline]
    pub fn literal(&self) -> Option<&Literal> {
        self.literal.as_ref()
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
            // Bit-parallel NFA: replaces add_epsilon stack with table lookups.
            if let Some(bp) = &self.bp_nfa {
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
                // a. Reset stale values for bits that are newly active (0->1).
                let newly = next & !current;
                let mut nb = newly;
                while nb != 0 {
                    let j = nb.trailing_zeros() as usize;
                    nb &= nb - 1;
                    origins[j] = usize::MAX;
                }
                // b. Propagate min origin from every source to every target in
                //    `next`.  Must cover ALL targets (not only newly-active ones)
                //    because back-edges (e.g. the loop in `(ab)+`) can deliver a
                //    smaller origin to an already-active state.
                let mut active = current;
                while active != 0 {
                    let i = active.trailing_zeros() as usize;
                    active &= active - 1;
                    let tgt = unsafe { *tbl.get_unchecked(i * 256 + byte as usize) };
                    let tgt_in_next = tgt & next;
                    if tgt_in_next != 0 {
                        let orig_i = origins[i];
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

// -- Helpers -------------------------------------------------------------------

/// Static class check used during bit-parallel table precomputation.
fn matches_class_static(class: &CharClass, c: char, ic: bool, dotall: bool) -> bool {
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
