use super::nfa::{Nfa, State};
use crate::captures::Match;
use crate::haystack::Haystack;

/// Active state with origin tracking
/// This is the canonical PikeVM representation:
/// each active state knows where it was spawned from
#[derive(Clone, Copy)]
struct ActiveState {
    /// NFA state index (program counter)
    pc: usize,
    /// Position where this thread was spawned (match start)
    origin: usize,
}

/// Thread list for PikeVM execution
/// Stores active states with their origins
struct ThreadList {
    /// Active states with origins
    states: Vec<ActiveState>,
    /// Deduplication: true if state is currently active (1 byte per state!)
    seen: Vec<bool>,
}

impl ThreadList {
    fn new(capacity: usize) -> Self {
        Self {
            states: Vec::with_capacity(capacity),
            seen: vec![false; capacity],
        }
    }

    fn clear(&mut self) {
        for state in &self.states {
            self.seen[state.pc] = false;
        }
        self.states.clear();
    }

    fn contains(&self, pc: usize) -> bool {
        self.seen[pc]
    }

    /// Insert state with origin. First insertion wins (earliest origin).
    /// O(1) - simple boolean check
    #[inline(always)]
    fn insert(&mut self, pc: usize, origin: usize) {
        if !self.seen[pc] {
            self.seen[pc] = true;
            self.states.push(ActiveState { pc, origin });
        }
        // If already seen, skip - first insertion wins (leftmost match)
    }

    fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// Get origin for a state (only used for match checking, so O(n) is acceptable)
    fn get_origin(&self, pc: usize) -> Option<usize> {
        for state in &self.states {
            if state.pc == pc {
                return Some(state.origin);
            }
        }
        None
    }
}

/// Reusable context for PikeVM execution
/// Allocated ONCE per PikeVM instance, reused across all find calls
struct VMContext {
    current: ThreadList,
    next: ThreadList,
    epsilon_stack: Vec<(usize, usize)>, // (pc, origin)
}

impl VMContext {
    fn new(num_states: usize) -> Self {
        Self {
            current: ThreadList::new(num_states),
            next: ThreadList::new(num_states),
            epsilon_stack: Vec::with_capacity(num_states),
        }
    }

    fn reset(&mut self) {
        self.current.clear();
        self.next.clear();
        self.epsilon_stack.clear();
    }
}

use std::sync::Mutex;

pub struct PikeVM {
    nfa: Nfa,
    start_byte: Option<u8>,
    ctx: Mutex<VMContext>,
}

#[cfg(feature = "internal_metrics")]
#[derive(Debug, Default)]
pub struct Metrics {
    pub steps: usize,
    pub active_states: usize,
    pub clones: usize,
}

impl PikeVM {
    pub fn new(nfa: Nfa, start_byte: Option<u8>) -> Self {
        let num_states = nfa.states.len();
        Self {
            nfa,
            start_byte,
            ctx: Mutex::new(VMContext::new(num_states)),
        }
    }

    pub fn find_from<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        self.find_raw(text, start_index)
    }

    pub fn find_raw<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        if self.start_byte.is_some() {
            self.find_raw_prefilter(text, start_index)
        } else {
            self.find_raw_baseline(text, start_index)
        }
    }

    /// Add state to current set with epsilon closure (stack-based, zero allocation)
    /// Takes individual field refs to allow split borrows
    fn add_state_epsilon<H: Haystack>(
        nfa: &Nfa,
        current: &mut ThreadList,
        epsilon_stack: &mut Vec<(usize, usize)>,
        state_id: usize,
        origin: usize,
        pos: usize,
        text: &H,
    ) {
        epsilon_stack.clear();
        epsilon_stack.push((state_id, origin));

        while let Some((sid, orig)) = epsilon_stack.pop() {
            if current.contains(sid) {
                // If already present, ThreadList.insert will keep earlier origin
                current.insert(sid, orig);
                continue;
            }

            current.insert(sid, orig);

            // Follow epsilon transitions, propagating origin unchanged
            match nfa.states[sid] {
                State::Jump(next) => {
                    epsilon_stack.push((next, orig));
                }
                State::Split(s1, s2) => {
                    epsilon_stack.push((s1, orig));
                    epsilon_stack.push((s2, orig));
                }
                State::Save(_, next) => {
                    epsilon_stack.push((next, orig));
                }
                State::AnchorStart(next) => {
                    if pos == 0 {
                        epsilon_stack.push((next, orig));
                    }
                }
                State::AnchorEnd(next) => {
                    if pos == text.len() {
                        epsilon_stack.push((next, orig));
                    }
                }
                State::WordBoundary(next) => {
                    if is_word_boundary(text, pos) {
                        epsilon_stack.push((next, orig));
                    }
                }
                State::WordStart(next) => {
                    if is_word_start(text, pos) {
                        epsilon_stack.push((next, orig));
                    }
                }
                State::WordEnd(next) => {
                    if is_word_end(text, pos) {
                        epsilon_stack.push((next, orig));
                    }
                }
                _ => {}
            }
        }
    }

    #[inline(always)]
    fn find_raw_prefilter<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        let mut guard = self.ctx.lock().unwrap();
        let ctx = &mut *guard; // Dereference to allow split borrows
        ctx.reset();

        let len = text.len();
        let sb = self.start_byte.unwrap();

        let mut best_match: Option<Match> = None;
        let mut pos = start_index;

        while pos <= len {
            // If no active states, jump to next start_byte
            if ctx.current.is_empty() && pos < len {
                if let Some(next_pos) = text.find_byte(sb, pos) {
                    pos = next_pos;
                } else {
                    break;
                }
            }

            // Try to start new match if we're at start_byte
            let can_start = pos < len && text.find_byte(sb, pos) == Some(pos);
            if can_start {
                let spawn_allowed = best_match.as_ref().map_or(true, |m| pos <= m.start);
                if spawn_allowed {
                    // Spawn new thread with origin = current position
                    Self::add_state_epsilon(
                        &self.nfa,
                        &mut ctx.current,
                        &mut ctx.epsilon_stack,
                        self.nfa.start,
                        pos, // origin
                        pos, // position for anchor checks
                        &text,
                    );
                }
            }

            // Check for match - get origin from match state
            if ctx.current.contains(self.nfa.match_state) {
                if let Some(origin) = ctx.current.get_origin(self.nfa.match_state) {
                    let new_match = Match {
                        start: origin,
                        end: pos,
                    };

                    let should_replace = match &best_match {
                        None => true,
                        Some(existing) => {
                            if new_match.start < existing.start {
                                true
                            } else if new_match.start == existing.start {
                                new_match.end > existing.end
                            } else {
                                false
                            }
                        }
                    };

                    if should_replace {
                        best_match = Some(new_match);
                    }
                }
            }

            if pos == len {
                break;
            }

            let (char_val, char_len) = match text.char_at(pos) {
                Some(c) => c,
                None => break,
            };

            // Step: transition current states to next states
            ctx.next.clear();

            // Iterate by index to avoid cloning - ZERO allocations
            for i in 0..ctx.current.states.len() {
                let state = ctx.current.states[i];
                if let Some(next_id) = self.get_next_state(state.pc, char_val) {
                    Self::add_state_to_next(
                        &self.nfa,
                        &mut ctx.next,
                        &mut ctx.epsilon_stack,
                        next_id,
                        state.origin, // First-seen origin is correct
                        pos + char_len,
                        &text,
                    );
                }
            }

            // Swap
            std::mem::swap(&mut ctx.current, &mut ctx.next);

            if best_match.is_some() && ctx.current.is_empty() {
                break;
            }

            pos += char_len;
        }

        best_match
    }

    /// Add state to NEXT set with epsilon closure (for character transitions)
    fn add_state_to_next<H: Haystack>(
        nfa: &Nfa,
        next: &mut ThreadList,
        stack: &mut Vec<(usize, usize)>,
        state_id: usize,
        origin: usize,
        pos: usize,
        text: &H,
    ) {
        stack.clear();
        stack.push((state_id, origin));

        while let Some((sid, orig)) = stack.pop() {
            if next.contains(sid) {
                // Update origin if new one is earlier
                next.insert(sid, orig);
                continue;
            }

            next.insert(sid, orig);

            match nfa.states[sid] {
                State::Jump(nxt) => {
                    stack.push((nxt, orig));
                }
                State::Split(s1, s2) => {
                    stack.push((s1, orig));
                    stack.push((s2, orig));
                }
                State::Save(_, nxt) => {
                    stack.push((nxt, orig));
                }
                State::AnchorStart(nxt) => {
                    if pos == 0 {
                        stack.push((nxt, orig));
                    }
                }
                State::AnchorEnd(nxt) => {
                    if pos == text.len() {
                        stack.push((nxt, orig));
                    }
                }
                State::WordBoundary(nxt) => {
                    if is_word_boundary(text, pos) {
                        stack.push((nxt, orig));
                    }
                }
                State::WordStart(nxt) => {
                    if is_word_start(text, pos) {
                        stack.push((nxt, orig));
                    }
                }
                State::WordEnd(nxt) => {
                    if is_word_end(text, pos) {
                        stack.push((nxt, orig));
                    }
                }
                _ => {}
            }
        }
    }

    #[inline(always)]
    fn find_raw_baseline<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        let mut guard = self.ctx.lock().unwrap();
        let ctx = &mut *guard; // Dereference to allow split borrows
        ctx.reset();

        let len = text.len();

        let mut best_match: Option<Match> = None;
        let mut pos = start_index;

        while pos <= len {
            // Spawn a new thread at this position if allowed
            let spawn_allowed = best_match.as_ref().map_or(true, |m| pos <= m.start);
            if spawn_allowed {
                Self::add_state_epsilon(
                    &self.nfa,
                    &mut ctx.current,
                    &mut ctx.epsilon_stack,
                    self.nfa.start,
                    pos, // origin
                    pos, // position for anchor checks
                    &text,
                );
            }

            // Check for match - get origin from match state
            if ctx.current.contains(self.nfa.match_state) {
                if let Some(origin) = ctx.current.get_origin(self.nfa.match_state) {
                    let new_match = Match {
                        start: origin,
                        end: pos,
                    };

                    let should_replace = match &best_match {
                        None => true,
                        Some(existing) => {
                            if new_match.start < existing.start {
                                true
                            } else if new_match.start == existing.start {
                                new_match.end > existing.end
                            } else {
                                false
                            }
                        }
                    };

                    if should_replace {
                        best_match = Some(new_match);
                    }
                }
            }

            if pos == len {
                break;
            }

            let (char_val, char_len) = match text.char_at(pos) {
                Some(c) => c,
                None => break,
            };

            // Step: transition current states to next states
            ctx.next.clear();

            // Iterate by index to avoid cloning - ZERO allocations
            for i in 0..ctx.current.states.len() {
                let state = ctx.current.states[i];
                if let Some(next_id) = self.get_next_state(state.pc, char_val) {
                    Self::add_state_to_next(
                        &self.nfa,
                        &mut ctx.next,
                        &mut ctx.epsilon_stack,
                        next_id,
                        state.origin, // First-seen origin is correct
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
            } // Dot matches anything but newline usually
            _ => None,
        }
    }

    fn match_class(&self, class: &crate::parser::CharClass, c: char) -> bool {
        // reuse logic? or implement simple check
        use crate::parser::CharClass::*;
        match class {
            Digit => c.is_ascii_digit(),
            NonDigit => !c.is_ascii_digit(),
            Word => c.is_alphanumeric() || c == '_',
            NonWord => !(c.is_alphanumeric() || c == '_'),
            Whitespace => c.is_whitespace(),
            NonWhitespace => !c.is_whitespace(),
            Dot => self.nfa.flags.dotall || c != '\n',
            Lowercase => {
                c.is_lowercase()
                    || (self.nfa.flags.ignore_case.unwrap_or(false) && c.is_uppercase())
            }
            NonLowercase => {
                !c.is_lowercase()
                    && (!self.nfa.flags.ignore_case.unwrap_or(false) || !c.is_uppercase())
            }
            Uppercase => {
                c.is_uppercase()
                    || (self.nfa.flags.ignore_case.unwrap_or(false) && c.is_lowercase())
            }
            NonUppercase => {
                !c.is_uppercase()
                    && (!self.nfa.flags.ignore_case.unwrap_or(false) || !c.is_lowercase())
            }
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
                let ignore_case = self.nfa.flags.ignore_case.unwrap_or(false);
                let found = chars.iter().any(|range| {
                    if c >= range.start && c <= range.end {
                        return true;
                    }
                    if ignore_case {
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

fn is_word_boundary<H: Haystack>(text: &H, pos: usize) -> bool {
    let prev = text.char_before(pos);
    let curr = text.char_at(pos).map(|(c, _)| c);
    is_word_char(prev) != is_word_char(curr)
}

fn is_word_start<H: Haystack>(text: &H, pos: usize) -> bool {
    let prev = text.char_before(pos);
    let curr = text.char_at(pos).map(|(c, _)| c);
    !is_word_char(prev) && is_word_char(curr)
}

fn is_word_end<H: Haystack>(text: &H, pos: usize) -> bool {
    let prev = text.char_before(pos);
    let curr = text.char_at(pos).map(|(c, _)| c);
    is_word_char(prev) && !is_word_char(curr)
}

fn is_word_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}
