use crate::flags::Flags;
use crate::parser::CharClass;

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Matches a specific byte/char.
    Char(char, usize),
    /// Matches a character class.
    Class(CharClass, usize),
    /// Matches any character (dot).
    Any(usize),
    /// Epsilon transition to one state.
    Jump(usize),
    /// Epsilon transition to two states (alternation/quantifier).
    Split(usize, usize),
    /// Save capture position for slot.
    Save(usize, usize),
    /// Word boundary (\b).
    WordBoundary(usize),
    /// Start of word boundary (\<).
    WordStart(usize),
    /// End of word boundary (\>).
    WordEnd(usize),
    /// Start Anchor (^).
    AnchorStart(usize),
    /// End Anchor ($).
    AnchorEnd(usize),
    /// End of match (accepting state).
    Match,
}

#[derive(Debug, Clone)]
pub struct Nfa {
    pub states: Vec<State>,
    pub start: usize,
    pub match_state: usize,
    pub flags: Flags,
}

impl Nfa {
    pub fn new() -> Self {
        Self {
            states: Vec::new(),
            start: 0,
            match_state: 0,
            flags: Flags::default(),
        }
    }

    pub fn add_state(&mut self, state: State) -> usize {
        let id = self.states.len();
        self.states.push(state);
        id
    }
}

impl Default for Nfa {
    fn default() -> Self {
        Self::new()
    }
}
