use super::nfa::{Nfa, State};
use crate::errors::CompileError;
use crate::flags::Flags;
use crate::parser::AstNode;

pub struct Compiler {
    nfa: Nfa,
    flags: Flags,
}

impl Compiler {
    pub fn new(flags: Flags) -> Self {
        Self {
            nfa: Nfa::new(),
            flags,
        }
    }

    pub fn compile(mut self, nodes: &[AstNode]) -> Result<Nfa, CompileError> {
        let (start, outs) = self.compile_slice(nodes)?;
        self.nfa.start = start;

        let match_state = self.nfa.add_state(State::Match);
        self.patch(outs, match_state);

        Ok(self.nfa)
    }

    fn compile_slice(&mut self, nodes: &[AstNode]) -> Result<(usize, Vec<usize>), CompileError> {
        if nodes.is_empty() {
            let s = self.nfa.add_state(State::Jump(0));
            return Ok((s, vec![s]));
        }

        let mut start = 0;
        let mut current_outs = Vec::new();

        for (i, node) in nodes.iter().enumerate() {
            let (s, outs) = self.compile_node(node)?;
            if i == 0 {
                start = s;
            } else {
                self.patch(current_outs, s);
            }
            current_outs = outs;
        }

        Ok((start, current_outs))
    }

    fn compile_node(&mut self, node: &AstNode) -> Result<(usize, Vec<usize>), CompileError> {
        match node {
            AstNode::Literal(c) => {
                if self.flags.ignore_case == Some(true) {
                    // Match case-insensitive
                    // Easiest is to create a Set of [lower, upper]
                    let lower = c.to_lowercase().next().unwrap_or(*c);
                    let upper = c.to_uppercase().next().unwrap_or(*c);

                    let range_lower = crate::parser::CharRange {
                        start: lower,
                        end: lower,
                    };
                    let range_upper = crate::parser::CharRange {
                        start: upper,
                        end: upper,
                    };

                    let class = crate::parser::CharClass::Set {
                        chars: vec![range_lower, range_upper],
                        negated: false,
                    };
                    let s = self.nfa.add_state(State::Class(class, 0));
                    Ok((s, vec![s]))
                } else {
                    let s = self.nfa.add_state(State::Char(*c, 0));
                    Ok((s, vec![s]))
                }
            }
            AstNode::CharClass(class) if matches!(class, crate::parser::CharClass::Dot) => {
                if self.flags.dotall {
                    let s = self.nfa.add_state(State::Class(
                        crate::parser::CharClass::Set {
                            chars: vec![],
                            negated: true,
                        },
                        0,
                    ));
                    Ok((s, vec![s]))
                } else {
                    let s = self.nfa.add_state(State::Any(0));
                    Ok((s, vec![s]))
                }
            }
            AstNode::CharClass(class) => {
                let s = self.nfa.add_state(State::Class(class.clone(), 0));
                Ok((s, vec![s]))
            }
            AstNode::StartAnchor => {
                let s = self.nfa.add_state(State::AnchorStart(0));
                Ok((s, vec![s]))
            }
            AstNode::EndAnchor => {
                let s = self.nfa.add_state(State::AnchorEnd(0));
                Ok((s, vec![s]))
            }
            AstNode::WordBoundary => {
                let s = self.nfa.add_state(State::WordBoundary(0));
                Ok((s, vec![s]))
            }
            AstNode::StartWord => {
                let s = self.nfa.add_state(State::WordStart(0));
                Ok((s, vec![s]))
            }
            AstNode::EndWord => {
                let s = self.nfa.add_state(State::WordEnd(0));
                Ok((s, vec![s]))
            }
            AstNode::Alternation(alts) => {
                if alts.is_empty() {
                    let s = self.nfa.add_state(State::Jump(0));
                    return Ok((s, vec![s]));
                }

                let mut alt_starts = Vec::new();
                let mut all_outs = Vec::new();

                for alt in alts {
                    let (s, outs) = self.compile_slice(alt)?;
                    alt_starts.push(s);
                    all_outs.extend(outs);
                }

                let mut start = *alt_starts.last().unwrap();
                for &s in alt_starts.iter().rev().skip(1) {
                    start = self.nfa.add_state(State::Split(s, start));
                }

                Ok((start, all_outs))
            }
            AstNode::ZeroOrMore { node, greedy: _ } => {
                let split = self.nfa.add_state(State::Split(0, 0));
                let (s, outs) = self.compile_node(node)?;

                // Patch Split(0) -> s
                match &mut self.nfa.states[split] {
                    State::Split(out1, _) => *out1 = s,
                    _ => unreachable!(),
                }

                // Patch Frag outs -> split (loop back)
                self.patch(outs, split);

                // Result start is split
                // Result outs is [split] (the second leg of split)
                Ok((split, vec![split]))
            }
            AstNode::OneOrMore { node, greedy: _ } => {
                // Frag -> Split(Frag, Out)
                let (s, outs) = self.compile_node(node)?;
                let split = self.nfa.add_state(State::Split(s, 0));
                self.patch(outs, split);

                Ok((s, vec![split]))
            }
            AstNode::Optional { node, greedy: _ } => {
                // Split(Frag, Out)
                let split = self.nfa.add_state(State::Split(0, 0));
                let (s, outs) = self.compile_node(node)?;

                match &mut self.nfa.states[split] {
                    State::Split(out1, _) => *out1 = s,
                    _ => unreachable!(),
                }

                let mut all_outs = outs;
                all_outs.push(split);
                Ok((split, all_outs))
            }
            AstNode::Group {
                nodes,
                name: _,
                capture,
                index,
            } => {
                if *capture {
                    if let Some(idx) = index {
                        // Slot 2*i is start, 2*i+1 is end
                        let start_slot = 2 * idx;
                        let end_slot = 2 * idx + 1;

                        // Start -> Save(start) -> Inner -> Save(end) -> Out

                        // We reserve states.
                        let save_start = self.nfa.add_state(State::Save(start_slot, 0));
                        let (inner_start, inner_outs) = self.compile_slice(nodes)?;

                        // Patch SaveStart -> Inner
                        match &mut self.nfa.states[save_start] {
                            State::Save(_, next) => *next = inner_start,
                            _ => unreachable!(),
                        }

                        // Create SaveEnd
                        let save_end = self.nfa.add_state(State::Save(end_slot, 0));

                        // Patch Inner outs -> SaveEnd
                        self.patch(inner_outs, save_end);

                        Ok((save_start, vec![save_end]))
                    } else {
                        // Capture but no index? Should not happen if parser is correct.
                        self.compile_slice(nodes)
                    }
                } else {
                    self.compile_slice(nodes)
                }
            }
            AstNode::Exact { node, count } => {
                if *count == 0 {
                    let s = self.nfa.add_state(State::Jump(0));
                    return Ok((s, vec![s]));
                }

                let mut start = 0;
                let mut current_outs = Vec::new();

                for i in 0..*count {
                    let (s, outs) = self.compile_node(node)?;
                    if i == 0 {
                        start = s;
                    } else {
                        self.patch(current_outs, s);
                    }
                    current_outs = outs;
                }
                Ok((start, current_outs))
            }
            AstNode::Range {
                node,
                min,
                max,
                greedy: _,
            } => {
                // 1. Min required matches
                // 2. Max optional matches

                let (min_start, min_outs) = if *min > 0 {
                    let mut start = 0;
                    let mut outs = Vec::new();
                    for i in 0..*min {
                        let (s, o) = self.compile_node(node)?;
                        if i == 0 {
                            start = s;
                        } else {
                            self.patch(outs, s);
                        }
                        outs = o;
                    }
                    (start, outs)
                } else {
                    (0, Vec::new())
                };

                let (opt_start, opt_outs) = if let Some(max_count) = max {
                    let count = max_count - min;
                    if count == 0 {
                        // No optional part
                        (0, Vec::new())
                    } else {
                        let mut start = 0;
                        let mut pending_outs = Vec::new();

                        for i in 0..count {
                            let (node_start, node_outs) = self.compile_node(node)?;
                            let split = self.nfa.add_state(State::Split(node_start, 0));

                            if i == 0 {
                                start = split;
                            } else {
                                // Patch previous pending outs to this split
                                self.patch(pending_outs, split);
                            }

                            pending_outs = node_outs;
                            pending_outs.push(split);
                        }
                        (start, pending_outs)
                    }
                } else {
                    let split = self.nfa.add_state(State::Split(0, 0));
                    let (s, outs) = self.compile_node(node)?;

                    match &mut self.nfa.states[split] {
                        State::Split(out1, _) => *out1 = s,
                        _ => unreachable!(),
                    }
                    self.patch(outs, split);

                    (split, vec![split])
                };

                if *min > 0 {
                    if let Some(_) = max {
                        if Some(*min) == *max {
                            return Ok((min_start, min_outs));
                        }
                    }

                    let has_infinite = max.is_none();
                    let has_optional = if let Some(m) = max { *m > *min } else { false };

                    if has_infinite || has_optional {
                        // Patch min_outs to opt_start
                        self.patch(min_outs, opt_start);
                        Ok((min_start, opt_outs))
                    } else {
                        // Only min part
                        Ok((min_start, min_outs))
                    }
                } else {
                    if *max == Some(0) {
                        // Empty match
                        let s = self.nfa.add_state(State::Jump(0));
                        Ok((s, vec![s]))
                    } else {
                        Ok((opt_start, opt_outs))
                    }
                }
            }
            _ => Err(CompileError::InvalidPattern(
                "Unsupported node type for linear engine".into(),
            )),
        }
    }

    fn patch(&mut self, outs: Vec<usize>, dest: usize) {
        for state_idx in outs {
            match &mut self.nfa.states[state_idx] {
                State::Jump(next) => *next = dest,
                State::Split(out1, out2) => {
                    if *out1 == 0 {
                        *out1 = dest;
                    } else if *out2 == 0 {
                        *out2 = dest;
                    }
                }
                State::Char(_, next) => *next = dest,
                State::Class(_, next) => *next = dest,
                State::Any(next) => *next = dest,
                State::Save(_, next) => *next = dest,
                State::AnchorStart(next) => *next = dest,
                State::AnchorEnd(next) => *next = dest,
                State::WordBoundary(next) => *next = dest,
                State::WordStart(next) => *next = dest,
                State::WordEnd(next) => *next = dest,
                _ => {}
            }
        }
    }
}
