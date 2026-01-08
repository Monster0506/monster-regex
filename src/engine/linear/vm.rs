use super::nfa::{Nfa, State};
use crate::captures::Match;
use crate::haystack::Haystack;

pub struct PikeVM {
    nfa: Nfa,
}

impl PikeVM {
    pub fn new(nfa: Nfa) -> Self {
        Self { nfa }
    }

    pub fn find_from<H: Haystack>(&self, text: H, start_index: usize) -> Option<Match> {
        self.find_raw(text, start_index).map(|(m, _)| m)
    }

    pub fn find_raw<H: Haystack>(
        &self,
        text: H,
        start_index: usize,
    ) -> Option<(Match, Vec<Option<usize>>)> {
        let mut matched: Option<(Match, Vec<Option<usize>>)> = None;
        let num_states = self.nfa.states.len();
        // current_states: maps state_id -> Captures
        // Captures is a Vec<Option<usize>> where indices correspond to capture group slots.
        // Slot 0 is start of match.
        let mut current_states: Vec<Option<Vec<Option<usize>>>> = vec![None; num_states];
        let mut next_states: Vec<Option<Vec<Option<usize>>>> = vec![None; num_states];

        // Active list to avoid iterating all `num_states`
        let mut active_ids = Vec::with_capacity(num_states);
        let mut next_active_ids = Vec::with_capacity(num_states);

        let len = text.len();

        for pos in start_index..=len {
            // Add start seed
            // Start state matches at `pos`.
            // Capture 0 is `pos`.
            let mut start_caps = vec![None; 20];
            start_caps.resize(2, None); // At least 0 and 1.
            start_caps[0] = Some(pos);

            self.add_state(
                &mut current_states,
                &mut active_ids,
                self.nfa.start,
                start_caps,
                pos,
                &text,
            );

            // Check matches in current_states
            if let Some(caps) = &current_states[self.nfa.match_state] {
                // Determine start from caps[0]
                let start = caps.first().copied().flatten().unwrap_or(pos); // Default?
                // End is current pos.

                let new_match = Match { start, end: pos };
                let replace = if let Some((ref existing, _)) = matched {
                    if new_match.start < existing.start {
                        true
                    } else if new_match.start == existing.start {
                        // Longer is better
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
                    // Try transition
                    if let Some(next_sid) = self.get_next_state(sid, char_val) {
                        self.add_state(
                            &mut next_states,
                            &mut next_active_ids,
                            next_sid,
                            caps.clone(),
                            pos + char_len,
                            &text,
                        );
                    }
                }
            }

            // Swap
            std::mem::swap(&mut current_states, &mut next_states);
            std::mem::swap(&mut active_ids, &mut next_active_ids);

            if matched.is_some() && active_ids.is_empty() {
                // If we have a match, and all threads died, we are done.
                // (Because any future match would start at > matched.start)
                break;
            }
        }

        matched
    }

    fn add_state<H: Haystack>(
        &self,
        states: &mut Vec<Option<Vec<Option<usize>>>>,
        active: &mut Vec<usize>,
        sid: usize,
        mut captures: Vec<Option<usize>>,
        current_pos: usize,
        text: &H,
    ) {
        if states[sid].is_some() {
            return;
        }

        states[sid] = Some(captures.clone());

        active.push(sid);

        // Epsilon closure
        match &self.nfa.states[sid] {
            State::Jump(next) => self.add_state(states, active, *next, captures, current_pos, text),
            State::Split(s1, s2) => {
                self.add_state(states, active, *s1, captures.clone(), current_pos, text);
                self.add_state(states, active, *s2, captures, current_pos, text);
            }
            State::Save(slot, next) => {
                if *slot >= captures.len() {
                    captures.resize(*slot + 1, None);
                }
                captures[*slot] = Some(current_pos);
                self.add_state(states, active, *next, captures, current_pos, text);
            }
            State::AnchorStart(next) => {
                if current_pos == 0 {
                    self.add_state(states, active, *next, captures, current_pos, text);
                }
            }
            State::AnchorEnd(next) => {
                if current_pos == text.len() {
                    self.add_state(states, active, *next, captures, current_pos, text);
                }
            }
            State::WordBoundary(next) => {
                if is_word_boundary(text, current_pos) {
                    self.add_state(states, active, *next, captures, current_pos, text);
                }
            }
            State::WordStart(next) => {
                if is_word_start(text, current_pos) {
                    self.add_state(states, active, *next, captures, current_pos, text);
                }
            }
            State::WordEnd(next) => {
                if is_word_end(text, current_pos) {
                    self.add_state(states, active, *next, captures, current_pos, text);
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
