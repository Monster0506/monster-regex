use super::nfa::{Nfa, State};
use crate::captures::Match;
use crate::haystack::Haystack;
use std::rc::Rc;

pub struct PikeVM {
    nfa: Nfa,
    start_byte: Option<u8>,
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
        Self { nfa, start_byte }
    }

    pub fn find_from<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        self.find_raw(text, start_index).map(|(m, _)| m)
    }

    pub fn find_raw<H: Haystack>(
        &self,
        text: H,
        start_index: usize,
    ) -> Option<(Match, Vec<Option<usize>>)> {
        if self.start_byte.is_some() {
            self.find_raw_prefilter(text, start_index)
        } else {
            self.find_raw_baseline(text, start_index)
        }
    }

    #[inline(always)]
    fn find_raw_prefilter<H: Haystack>(
        &self,
        text: H,
        start_index: usize,
    ) -> Option<(Match, Vec<Option<usize>>)> {
        let mut matched: Option<(Match, Rc<Vec<Option<usize>>>)> = None;
        let num_states = self.nfa.states.len();
        let mut current_states: Vec<Option<Rc<Vec<Option<usize>>>>> = vec![None; num_states];
        let mut next_states: Vec<Option<Rc<Vec<Option<usize>>>>> = vec![None; num_states];
        let mut active_ids = Vec::with_capacity(num_states);
        let mut next_active_ids = Vec::with_capacity(num_states);
        let len = text.len();

        #[cfg(feature = "internal_metrics")]
        let mut metrics = Metrics::default();

        let mut pos = start_index;
        let sb = self.start_byte.unwrap(); // helper called only if some

        while pos <= len {
            // Optimization: If no active states, jump using start_byte
            if active_ids.is_empty() {
                if pos < len {
                    if let Some(next_pos) = text.find_byte(sb, pos) {
                        if next_pos > pos {
                            pos = next_pos;
                        }
                    } else {
                        break;
                    }
                }
            }

            // Start seed logic: match start_byte
            let can_start = pos < len && text.find_byte(sb, pos) == Some(pos);

            if can_start {
                let mut start_caps_vec = vec![None; 20];
                start_caps_vec.resize(2, None);
                start_caps_vec[0] = Some(pos);
                let start_caps = Rc::new(start_caps_vec);

                let spawn_allowed = matched.as_ref().map_or(true, |(m, _)| pos <= m.start);

                if spawn_allowed {
                    self.add_state(
                        &mut current_states,
                        &mut active_ids,
                        self.nfa.start,
                        start_caps,
                        pos,
                        &text,
                        #[cfg(feature = "internal_metrics")]
                        &mut metrics,
                    );
                }
            }

            // Check matches
            if let Some(caps) = &current_states[self.nfa.match_state] {
                let start = caps.first().copied().flatten().unwrap_or(pos);
                let new_match = Match { start, end: pos };
                let replace = if let Some((ref existing, _)) = matched {
                    if new_match.start < existing.start {
                        true
                    } else if new_match.start == existing.start {
                        new_match.end > existing.end
                    } else {
                        false
                    }
                } else {
                    true
                };
                if replace {
                    matched = Some((new_match, caps.clone()));
                }
            }

            if pos == len {
                break;
            }

            let (char_val, char_len) = match text.char_at(pos) {
                Some(c) => c,
                None => break,
            };

            // Step
            next_states.fill(None);
            next_active_ids.clear();
            for &sid in &active_ids {
                if let Some(caps) = &current_states[sid] {
                    if let Some(next_sid) = self.get_next_state(sid, char_val) {
                        self.add_state(
                            &mut next_states,
                            &mut next_active_ids,
                            next_sid,
                            caps.clone(),
                            pos + char_len,
                            &text,
                            #[cfg(feature = "internal_metrics")]
                            &mut metrics,
                        );
                        #[cfg(feature = "internal_metrics")]
                        {
                            metrics.clones += 1;
                        }
                    }
                }
            }

            #[cfg(feature = "internal_metrics")]
            {
                metrics.steps += 1;
                metrics.active_states += active_ids.len();
            }

            std::mem::swap(&mut current_states, &mut next_states);
            std::mem::swap(&mut active_ids, &mut next_active_ids);

            if matched.is_some() && active_ids.is_empty() {
                break;
            }

            pos += char_len;
        }

        matched.map(|(m, caps)| (m, (*caps).clone()))
    }

    #[inline(always)]
    fn find_raw_baseline<H: Haystack>(
        &self,
        text: H,
        start_index: usize,
    ) -> Option<(Match, Vec<Option<usize>>)> {
        let mut matched: Option<(Match, Rc<Vec<Option<usize>>>)> = None;
        let num_states = self.nfa.states.len();
        let mut current_states: Vec<Option<Rc<Vec<Option<usize>>>>> = vec![None; num_states];
        let mut next_states: Vec<Option<Rc<Vec<Option<usize>>>>> = vec![None; num_states];
        let mut active_ids = Vec::with_capacity(num_states);
        let mut next_active_ids = Vec::with_capacity(num_states);
        let len = text.len();

        #[cfg(feature = "internal_metrics")]
        let mut metrics = Metrics::default();

        let mut pos = start_index;
        while pos <= len {
            // Always try to spawn start seed
            // Start state matches at `pos`.
            let mut start_caps_vec = vec![None; 20];
            start_caps_vec.resize(2, None);
            start_caps_vec[0] = Some(pos);
            let start_caps = Rc::new(start_caps_vec);

            let spawn_allowed = matched.as_ref().map_or(true, |(m, _)| pos <= m.start);
            if spawn_allowed {
                self.add_state(
                    &mut current_states,
                    &mut active_ids,
                    self.nfa.start,
                    start_caps,
                    pos,
                    &text,
                    #[cfg(feature = "internal_metrics")]
                    &mut metrics,
                );
            }

            // Check matches
            if let Some(caps) = &current_states[self.nfa.match_state] {
                let start = caps.first().copied().flatten().unwrap_or(pos);
                let new_match = Match { start, end: pos };
                let replace = if let Some((ref existing, _)) = matched {
                    if new_match.start < existing.start {
                        true
                    } else if new_match.start == existing.start {
                        new_match.end > existing.end
                    } else {
                        false
                    }
                } else {
                    true
                };
                if replace {
                    matched = Some((new_match, caps.clone()));
                }
            }

            if pos == len {
                break;
            }

            let (char_val, char_len) = match text.char_at(pos) {
                Some(c) => c,
                None => break,
            };

            // Step
            next_states.fill(None);
            next_active_ids.clear();
            for &sid in &active_ids {
                if let Some(caps) = &current_states[sid] {
                    if let Some(next_sid) = self.get_next_state(sid, char_val) {
                        self.add_state(
                            &mut next_states,
                            &mut next_active_ids,
                            next_sid,
                            caps.clone(),
                            pos + char_len,
                            &text,
                            #[cfg(feature = "internal_metrics")]
                            &mut metrics,
                        );
                        #[cfg(feature = "internal_metrics")]
                        {
                            metrics.clones += 1;
                        }
                    }
                }
            }

            #[cfg(feature = "internal_metrics")]
            {
                metrics.steps += 1;
                metrics.active_states += active_ids.len();
            }

            std::mem::swap(&mut current_states, &mut next_states);
            std::mem::swap(&mut active_ids, &mut next_active_ids);

            if matched.is_some() && active_ids.is_empty() {
                break;
            }

            pos += char_len;
        }

        #[cfg(feature = "internal_metrics")]
        eprintln!(
            "PROFILE: len={} steps={} avg_states={:.2} total_clones={} nfa_size={}",
            len,
            metrics.steps,
            metrics.active_states as f64 / metrics.steps.max(1) as f64,
            metrics.clones,
            self.nfa.states.len()
        );

        matched.map(|(m, caps)| (m, (*caps).clone()))
    }

    fn add_state<H: Haystack>(
        &self,
        states: &mut Vec<Option<Rc<Vec<Option<usize>>>>>,
        active: &mut Vec<usize>,
        sid: usize,
        captures: Rc<Vec<Option<usize>>>,
        current_pos: usize,
        text: &H,
        #[cfg(feature = "internal_metrics")] metrics: &mut Metrics,
    ) {
        if states[sid].is_some() {
            return;
        }

        states[sid] = Some(captures.clone());

        active.push(sid);

        // Epsilon closure
        match &self.nfa.states[sid] {
            State::Jump(next) => self.add_state(
                states,
                active,
                *next,
                captures,
                current_pos,
                text,
                #[cfg(feature = "internal_metrics")]
                metrics,
            ),
            State::Split(s1, s2) => {
                self.add_state(
                    states,
                    active,
                    *s1,
                    captures.clone(),
                    current_pos,
                    text,
                    #[cfg(feature = "internal_metrics")]
                    metrics,
                );
                #[cfg(feature = "internal_metrics")]
                {
                    metrics.clones += 1; // Count RC clone as clone
                }
                self.add_state(
                    states,
                    active,
                    *s2,
                    captures,
                    current_pos,
                    text,
                    #[cfg(feature = "internal_metrics")]
                    metrics,
                );
            }
            State::Save(slot, next) => {
                // COW: Make mutable. If strictly owned, no clone. If shared, clones vec.
                let mut caps = captures;
                let inner = Rc::make_mut(&mut caps);

                if *slot >= inner.len() {
                    inner.resize(*slot + 1, None);
                }
                inner[*slot] = Some(current_pos);

                self.add_state(
                    states,
                    active,
                    *next,
                    caps,
                    current_pos,
                    text,
                    #[cfg(feature = "internal_metrics")]
                    metrics,
                );
            }
            State::AnchorStart(next) => {
                if current_pos == 0 {
                    self.add_state(
                        states,
                        active,
                        *next,
                        captures,
                        current_pos,
                        text,
                        #[cfg(feature = "internal_metrics")]
                        metrics,
                    );
                }
            }
            State::AnchorEnd(next) => {
                if current_pos == text.len() {
                    self.add_state(
                        states,
                        active,
                        *next,
                        captures,
                        current_pos,
                        text,
                        #[cfg(feature = "internal_metrics")]
                        metrics,
                    );
                }
            }
            State::WordBoundary(next) => {
                if is_word_boundary(text, current_pos) {
                    self.add_state(
                        states,
                        active,
                        *next,
                        captures,
                        current_pos,
                        text,
                        #[cfg(feature = "internal_metrics")]
                        metrics,
                    );
                }
            }
            State::WordStart(next) => {
                if is_word_start(text, current_pos) {
                    self.add_state(
                        states,
                        active,
                        *next,
                        captures,
                        current_pos,
                        text,
                        #[cfg(feature = "internal_metrics")]
                        metrics,
                    );
                }
            }
            State::WordEnd(next) => {
                if is_word_end(text, current_pos) {
                    self.add_state(
                        states,
                        active,
                        *next,
                        captures,
                        current_pos,
                        text,
                        #[cfg(feature = "internal_metrics")]
                        metrics,
                    );
                }
            }
            _ => {}
        }
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
