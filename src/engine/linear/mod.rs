pub mod compiler;
pub mod nfa;
mod tests;
pub mod vm;

use crate::captures::{Captures, Match};
use crate::engine::{CompiledRegex, CompiledRegexHaystack, RegexEngine};
use crate::errors::CompileError;
use crate::flags::Flags;
use crate::haystack::Haystack;
use crate::parser::{AstNode, Parser};
use compiler::Compiler;
use std::collections::HashMap;
use vm::PikeVM;

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
                if *capture {
                    if let Some(i) = index {
                        *count = (*count).max(*i);
                        if let Some(n) = name {
                            map.insert(n.clone(), *i);
                        }
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

/// A compiled regex using the linear engine.
pub struct LinearRegex {
    vm: PikeVM,
    pattern: String,
    flags: Flags,
    group_count: usize,
    named_groups: HashMap<String, usize>,
}

impl LinearRegex {
    /// Compiles a new linear regex.
    pub fn new(pattern: &str, mut flags: Flags) -> Result<Self, CompileError> {
        // Smartcase
        if flags.ignore_case.is_none() {
            let has_uppercase = pattern.chars().any(|c| c.is_uppercase());
            flags.ignore_case = Some(!has_uppercase);
        }

        let mut parser = Parser::new(pattern, flags);
        let ast = parser
            .parse()
            .map_err(|e| CompileError::InvalidPattern(e.to_string()))?;

        // Extract capture group names and count
        let (group_count, named_groups) = analyze_captures(&ast);

        // Analyze start byte for properties optimization
        let start_byte = analyze_start_byte(&ast, &flags);

        let compiler = Compiler::new(flags);
        let nfa = compiler.compile(&ast)?;
        let vm = PikeVM::new(nfa, start_byte);

        Ok(LinearRegex {
            vm,
            pattern: pattern.to_string(),
            flags,
            group_count,
            named_groups,
        })
    }
}

fn analyze_start_byte(nodes: &[AstNode], flags: &Flags) -> Option<u8> {
    if nodes.is_empty() {
        return None;
    }

    let ic = flags.ignore_case.unwrap_or(false);

    match &nodes[0] {
        AstNode::Literal(c) => {
            if ic && c.to_lowercase().next() != c.to_uppercase().next() {
                return None;
            }
            if c.is_ascii() {
                return Some(*c as u8);
            }
        }
        AstNode::Exact { node, .. } | AstNode::OneOrMore { node, .. } => {
            if let AstNode::Literal(c) = &**node {
                if ic && c.to_lowercase().next() != c.to_uppercase().next() {
                    return None;
                }
                if c.is_ascii() {
                    return Some(*c as u8);
                }
            }
            // Recursively analyze group if wrapped
            if let AstNode::Group { nodes, .. } = &**node {
                return analyze_start_byte(nodes, flags);
            }
        }
        AstNode::Group { nodes, .. } => {
            return analyze_start_byte(nodes, flags);
        }
        _ => {}
    }
    None
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
        Box::new(FindMatchesIterator {
            text,
            regex: self,
            last_end: 0,
        })
    }

    fn captures(&self, text: &str) -> Option<Captures> {
        // TODO: Greedy mode doesn't track captures during search
        // Captures are NOT supported in the Linear engine yet
        // For now, return basic match without capture groups
        let full = self.vm.find_raw(text, 0)?;

        Some(Captures {
            full_match: full,
            groups: vec![], // Empty - captures not supported
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

// Iterator implementations for linear
// TODO: reduce duplication with backtracking/regex mod
struct FindMatchesIterator<'a, H: Haystack> {
    text: H,
    regex: &'a LinearRegex,
    last_end: usize,
}

impl<'a, H: Haystack> Iterator for FindMatchesIterator<'a, H> {
    type Item = Match;

    fn next(&mut self) -> Option<Self::Item> {
        if self.last_end > self.text.len() {
            return None;
        }
        let m = self.regex.find_from_at(self.text.clone(), self.last_end)?;
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
        // TODO: optimize valid start search
        // We iterate char by char if we don't have find_at equivalent for captures?
        // But we DO have regex.captures(text_slice).
        // Wait, captures() matches the *first* match.
        // We need find_at() equivalent that returns Captures.
        // LinearRegex::captures() implemented above uses find_raw(text, 0).
        // We need find_raw(text, self.last_end).
        // But captures() API on CompiledRegex currently takes only &str (impl detail above is hardcoded).
        // The trait definition: fn captures(&self, text: &str) -> Option<Captures>;
        // It doesn't have start pos.
        // So we must slice the text: &text[self.last_end..].
        // But then indices in Captures are relative to slice!
        // We must adjust them.

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
