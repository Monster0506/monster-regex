use crate::captures::Match;
use crate::flags::Flags;
use crate::parser::{AstNode, CharClass};

use crate::haystack::{Haystack, HaystackCursor};

/// The matching engine that walks the AST to find matches in text.
pub struct Matcher<'a, H: Haystack> {
    nodes: &'a [AstNode],
    flags: &'a Flags,
    text: H,
}

struct QuantifierParams {
    min: usize,
    max: Option<usize>,
    greedy: bool,
}

#[derive(Clone, Debug)]
struct MatchContext {
    captures: Vec<Option<Match>>,
    match_start_override: Option<usize>,
    match_end_override: Option<usize>,
}

impl MatchContext {
    fn new(group_count: usize) -> Self {
        Self {
            captures: vec![None; group_count + 1], // +1 for 1-based indexing
            match_start_override: None,
            match_end_override: None,
        }
    }

    fn clear(&mut self) {
        self.captures.fill(None);
        self.match_start_override = None;
        self.match_end_override = None;
    }
}

impl<'a, H: Haystack> Matcher<'a, H> {
    /// Creates a new Matcher instance.
    pub fn new(nodes: &'a [AstNode], flags: &'a Flags, text: H) -> Self {
        Self { nodes, flags, text }
    }

    /// Finds the first match in the text.
    pub fn find(&self) -> Option<Match> {
        self.find_at(0)
    }

    /// Finds the first match in the text starting at the given position.
    pub fn find_at(&self, start_index: usize) -> Option<Match> {
        // Determine max group index for context sizing
        let max_group = self.count_groups(self.nodes);

        // Try to match starting at every character boundary
        let mut pos = start_index;
        let len = self.text.len();

        let mut cursor = self.text.cursor_at(pos);
        let mut prev_char = if pos > 0 {
            self.text.char_before(pos)
        } else {
            None
        };

        let mut context = MatchContext::new(max_group);

        while pos <= len {
            // Reset context for this attempt
            context.clear();

            // We need a clone of cursor for this attempt
            let mut match_cursor = cursor.clone();

            if let Some(end_pos) =
                self.match_nodes(self.nodes, pos, &mut context, &mut match_cursor, prev_char)
            {
                let start = context.match_start_override.unwrap_or(pos);
                let end = context.match_end_override.unwrap_or(end_pos);
                return Some(Match { start, end });
            }

            // Advance to next char
            if pos < len {
                if let Some(c) = cursor.next() {
                    prev_char = Some(c);
                    pos += c.len_utf8();
                } else {
                    pos += 1; // Should not happen
                }
            } else {
                break;
            }
        }

        None
    }

    // Helper to count groups to size the capture vector
    fn count_groups(&self, nodes: &[AstNode]) -> usize {
        let mut max = 0;
        for node in nodes {
            match node {
                AstNode::Group { index, nodes, .. } => {
                    if let Some(i) = index {
                        max = max.max(*i);
                    }
                    max = max.max(self.count_groups(nodes));
                }
                AstNode::Alternation(alts) => {
                    for alt in alts {
                        max = max.max(self.count_groups(alt));
                    }
                }
                AstNode::ZeroOrMore { node, .. }
                | AstNode::OneOrMore { node, .. }
                | AstNode::Optional { node, .. }
                | AstNode::Exact { node, .. }
                | AstNode::Range { node, .. } => {
                    max = max.max(self.count_groups(std::slice::from_ref(node)));
                }
                AstNode::LookAhead { nodes, .. } | AstNode::LookBehind { nodes, .. } => {
                    max = max.max(self.count_groups(nodes));
                }
                _ => {}
            }
        }
        max
    }

    fn match_nodes(
        &self,
        nodes: &[AstNode],
        pos: usize,
        ctx: &mut MatchContext,
        cursor: &mut H::Cursor,
        prev_char: Option<char>, // Optimization: pass previous char to avoid potentially expensive char_before()
    ) -> Option<usize> {
        if nodes.is_empty() {
            return Some(pos);
        }

        let node = &nodes[0];
        let remaining = &nodes[1..];

        match node {
            AstNode::Literal(c) => {
                let mut temp_cursor = cursor.clone();
                let current_char = temp_cursor.next()?;
                let char_len = current_char.len_utf8();

                let matches = if self.flags.ignore_case.unwrap_or(false) {
                    c.to_lowercase().eq(current_char.to_lowercase())
                } else {
                    current_char == *c
                };

                if matches {
                    let next_pos = pos + char_len;
                    *cursor = temp_cursor;
                    self.match_nodes(remaining, next_pos, ctx, cursor, Some(current_char))
                } else {
                    None
                }
            }
            AstNode::CharClass(class) => {
                let mut temp_cursor = cursor.clone();
                let current_char = temp_cursor.next()?;
                let len = current_char.len_utf8();
                if self.match_char_class(class, current_char) {
                    *cursor = temp_cursor;
                    self.match_nodes(remaining, pos + len, ctx, cursor, Some(current_char))
                } else {
                    None
                }
            }
            AstNode::StartAnchor => {
                let is_start = pos == 0;
                let is_line_start = self.flags.multiline && pos > 0 && prev_char == Some('\n');
                if is_start || is_line_start {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char)
                } else {
                    None
                }
            }
            AstNode::EndAnchor => {
                let is_end = pos == self.text.len();
                let is_line_end =
                    self.flags.multiline && pos < self.text.len() && cursor.peek() == Some('\n');
                if is_end || is_line_end {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char)
                } else {
                    None
                }
            }
            AstNode::WordBoundary => {
                if self.is_word_boundary(cursor, prev_char) {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char)
                } else {
                    None
                }
            }
            AstNode::StartWord => {
                if self.is_word_boundary(cursor, prev_char) && self.is_word_char_at(cursor) {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char)
                } else {
                    None
                }
            }
            AstNode::EndWord => {
                if self.is_word_boundary(cursor, prev_char) && !self.is_word_char_at(cursor) {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char)
                } else {
                    None
                }
            }
            AstNode::SetMatchStart => {
                ctx.match_start_override = Some(pos);
                self.match_nodes(remaining, pos, ctx, cursor, prev_char)
            }
            AstNode::SetMatchEnd => {
                ctx.match_end_override = Some(pos);
                self.match_nodes(remaining, pos, ctx, cursor, prev_char)
            }
            AstNode::Alternation(alts) => {
                for alt in alts {
                    let mut fork_ctx = ctx.clone();
                    let mut fork_cursor = cursor.clone();

                    if let Some(next_pos) =
                        self.match_nodes(alt, pos, &mut fork_ctx, &mut fork_cursor, prev_char)
                    {
                        // Note: Alternation children update prev_char internally if they consume.
                        // We must pass the correct prev_char to 'remaining'.
                        // But wait, `next_pos` might be > `pos`, meaning we consumed chars.
                        // We strictly need the `prev_char` AT `next_pos`.
                        // But `match_nodes` API relies on us determining it?
                        // If `alt` consumed chars, `fork_cursor` advanced.
                        // We don't have direct access to the char before `fork_cursor` without backtracking or tracking return.
                        // However, we can use `self.text.char_before(next_pos)` here. Since alternations/groups are higher level steps,
                        // calling it once per group match is much cheaper than per character in looping.
                        // BUT, ideally we pass it back.
                        // For now, let's use `char_before` here as it is safer and much less frequent than the inner loop.
                        let next_prev_char = if next_pos > pos {
                            self.text.char_before(next_pos)
                        } else {
                            prev_char
                        };

                        if let Some(final_pos) = self.match_nodes(
                            remaining,
                            next_pos,
                            &mut fork_ctx,
                            &mut fork_cursor,
                            next_prev_char,
                        ) {
                            *ctx = fork_ctx;
                            *cursor = fork_cursor;
                            return Some(final_pos);
                        }
                    }
                }
                None
            }
            AstNode::Group {
                nodes: group_nodes,
                capture,
                index,
                ..
            } => {
                let start_capture = pos;
                let mut group_cursor = cursor.clone();

                if let Some(next_pos) =
                    self.match_nodes(group_nodes, pos, ctx, &mut group_cursor, prev_char)
                {
                    if *capture && index.is_some() {
                        let idx = index.unwrap();
                        if idx < ctx.captures.len() {
                            ctx.captures[idx] = Some(Match {
                                start: start_capture,
                                end: next_pos,
                            });
                        }
                    }

                    let next_prev_char = if next_pos > pos {
                        self.text.char_before(next_pos)
                    } else {
                        prev_char
                    };

                    if let Some(end_pos) = self.match_nodes(
                        remaining,
                        next_pos,
                        ctx,
                        &mut group_cursor,
                        next_prev_char,
                    ) {
                        *cursor = group_cursor;
                        Some(end_pos)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            AstNode::Backref(idx) => {
                if let Some(Some(m)) = ctx.captures.get(*idx) {
                    if self.text.matches_range(pos, m.start, m.end) {
                        let len_to_skip = m.end - m.start;
                        // let mut temp_pos = pos; // Removed unused variable
                        let mut temp_cursor = cursor.clone();

                        let target_pos = pos + len_to_skip;
                        let mut current_byte_pos = pos;
                        let mut last_char = prev_char;

                        while current_byte_pos < target_pos {
                            if let Some(c) = temp_cursor.next() {
                                last_char = Some(c);
                                current_byte_pos += c.len_utf8();
                            } else {
                                return None;
                            }
                        }

                        *cursor = temp_cursor;
                        self.match_nodes(remaining, target_pos, ctx, cursor, last_char)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            AstNode::LookAhead {
                nodes: look_nodes,
                positive,
            } => {
                let mut look_ctx = ctx.clone();
                let mut look_cursor = cursor.clone();
                let matched = self
                    .match_nodes(look_nodes, pos, &mut look_ctx, &mut look_cursor, prev_char)
                    .is_some();
                if matched == *positive {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char)
                } else {
                    None
                }
            }
            AstNode::LookBehind {
                nodes: look_nodes,
                positive,
            } => {
                // Lookbehind implementation: try matching ending at pos
                // Lookbehind doesn't consume, so prev_char is irrelevant for the next step?
                // But wait, inside lookbehind we match backwards or simulate it.
                // Current impl simulates by trying all start positions ending at pos.
                let mut matched = false;
                // Optimization: If lookbehind is fixed length, we only check one spot.
                // But general case requires loop.
                for start in 0..=pos {
                    let mut look_ctx = ctx.clone();
                    let mut look_cursor = self.text.cursor_at(start);
                    // For the inner match, we need prev_char at 'start'. Expensive?
                    // Yes, but LookBehind is generally expensive.
                    let start_prev_char = if start > 0 {
                        self.text.char_before(start)
                    } else {
                        None
                    };

                    if let Some(end) = self.match_nodes(
                        look_nodes,
                        start,
                        &mut look_ctx,
                        &mut look_cursor,
                        start_prev_char,
                    ) && end == pos
                    {
                        matched = true;
                        break;
                    }
                }

                if matched == *positive {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char)
                } else {
                    None
                }
            }
            AstNode::ZeroOrMore {
                node: inner,
                greedy,
            } => self.match_quantifier(
                inner,
                QuantifierParams {
                    min: 0,
                    max: None,
                    greedy: *greedy,
                },
                remaining,
                pos,
                ctx,
                cursor,
                prev_char,
            ),
            AstNode::OneOrMore {
                node: inner,
                greedy,
            } => self.match_quantifier(
                inner,
                QuantifierParams {
                    min: 1,
                    max: None,
                    greedy: *greedy,
                },
                remaining,
                pos,
                ctx,
                cursor,
                prev_char,
            ),
            AstNode::Optional {
                node: inner,
                greedy,
            } => self.match_quantifier(
                inner,
                QuantifierParams {
                    min: 0,
                    max: Some(1),
                    greedy: *greedy,
                },
                remaining,
                pos,
                ctx,
                cursor,
                prev_char,
            ),
            AstNode::Exact { node: inner, count } => self.match_quantifier(
                inner,
                QuantifierParams {
                    min: *count,
                    max: Some(*count),
                    greedy: true,
                },
                remaining,
                pos,
                ctx,
                cursor,
                prev_char,
            ),
            AstNode::Range {
                node: inner,
                min,
                max,
                greedy,
            } => self.match_quantifier(
                inner,
                QuantifierParams {
                    min: *min,
                    max: *max,
                    greedy: *greedy,
                },
                remaining,
                pos,
                ctx,
                cursor,
                prev_char,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn match_quantifier(
        &self,
        node: &AstNode,
        params: QuantifierParams,
        remaining: &[AstNode],
        pos: usize,
        ctx: &mut MatchContext,
        cursor: &mut H::Cursor,
        mut prev_char: Option<char>,
    ) -> Option<usize> {
        // 1. Match minimum required times
        let mut curr_pos = pos;
        for _ in 0..params.min {
            if let Some(next_pos) =
                self.match_nodes(std::slice::from_ref(node), curr_pos, ctx, cursor, prev_char)
            {
                // Update prev_char for next iteration
                if next_pos > curr_pos {
                    prev_char = self.text.char_before(next_pos);
                }
                curr_pos = next_pos;
            } else {
                return None;
            }
        }

        // 2. Match optional times
        self.match_quantifier_optional(
            node,
            params.max.map(|m| m - params.min),
            params.greedy,
            remaining,
            curr_pos,
            ctx,
            cursor,
            prev_char,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn match_quantifier_optional(
        &self,
        node: &AstNode,
        max_remaining: Option<usize>,
        greedy: bool,
        remaining: &[AstNode],
        pos: usize,
        ctx: &mut MatchContext,
        cursor: &mut H::Cursor,
        prev_char: Option<char>,
    ) -> Option<usize> {
        if let Some(0) = max_remaining {
            return self.match_nodes(remaining, pos, ctx, cursor, prev_char);
        }

        if greedy {
            // Try to match one more
            let mut fork_ctx = ctx.clone();
            let mut fork_cursor = cursor.clone();

            if let Some(next_pos) = self.match_nodes(
                std::slice::from_ref(node),
                pos,
                &mut fork_ctx,
                &mut fork_cursor,
                prev_char,
            ) {
                let next_prev_char = if next_pos > pos {
                    self.text.char_before(next_pos)
                } else {
                    prev_char
                };

                // Prevent infinite loops on zero-width matches
                if next_pos > pos
                    && let Some(final_pos) = self.match_quantifier_optional(
                        node,
                        max_remaining.map(|m| m - 1),
                        greedy,
                        remaining,
                        next_pos,
                        &mut fork_ctx,
                        &mut fork_cursor,
                        next_prev_char,
                    )
                {
                    *ctx = fork_ctx;
                    *cursor = fork_cursor;
                    return Some(final_pos);
                }
            }

            // If we couldn't match more, or the recursive call failed, try matching the rest
            self.match_nodes(remaining, pos, ctx, cursor, prev_char)
        } else {
            // Lazy: Try matching the rest first
            let mut fork_ctx = ctx.clone();
            let mut fork_cursor = cursor.clone();
            if let Some(final_pos) =
                self.match_nodes(remaining, pos, &mut fork_ctx, &mut fork_cursor, prev_char)
            {
                *ctx = fork_ctx;
                *cursor = fork_cursor;
                return Some(final_pos);
            }

            // If that fails, try matching one more
            if let Some(next_pos) =
                self.match_nodes(std::slice::from_ref(node), pos, ctx, cursor, prev_char)
                && next_pos > pos
            {
                let next_prev_char = self.text.char_before(next_pos);
                return self.match_quantifier_optional(
                    node,
                    max_remaining.map(|m| m - 1),
                    greedy,
                    remaining,
                    next_pos,
                    ctx,
                    cursor,
                    next_prev_char,
                );
            }
            None
        }
    }

    fn match_char_class(&self, class: &CharClass, c: char) -> bool {
        match class {
            CharClass::Digit => c.is_ascii_digit(),
            CharClass::NonDigit => !c.is_ascii_digit(),
            CharClass::Word => c.is_alphanumeric() || c == '_',
            CharClass::NonWord => !(c.is_alphanumeric() || c == '_'),
            CharClass::Whitespace => c.is_whitespace(),
            CharClass::NonWhitespace => !c.is_whitespace(),
            CharClass::Dot => self.flags.dotall || c != '\n',
            CharClass::Lowercase => {
                c.is_lowercase() || (self.flags.ignore_case.unwrap_or(false) && c.is_uppercase())
            }
            CharClass::NonLowercase => {
                !c.is_lowercase() && (!self.flags.ignore_case.unwrap_or(false) || !c.is_uppercase())
            }
            CharClass::Uppercase => {
                c.is_uppercase() || (self.flags.ignore_case.unwrap_or(false) && c.is_lowercase())
            }
            CharClass::NonUppercase => {
                !c.is_uppercase() && (!self.flags.ignore_case.unwrap_or(false) || !c.is_lowercase())
            }
            CharClass::Hex => c.is_ascii_hexdigit(),
            CharClass::NonHex => !c.is_ascii_hexdigit(),
            CharClass::Octal => c.is_digit(8),
            CharClass::NonOctal => !c.is_digit(8),
            CharClass::Alphanumeric => c.is_alphanumeric(),
            CharClass::NonAlphanumeric => !c.is_alphanumeric(),
            CharClass::Punctuation => c.is_ascii_punctuation(),
            CharClass::NonPunctuation => !c.is_ascii_punctuation(),
            CharClass::WordStart => c.is_alphabetic() || c == '_',
            CharClass::NonWordStart => !(c.is_alphabetic() || c == '_'),
            CharClass::Set { chars, negated } => {
                let ignore_case = self.flags.ignore_case.unwrap_or(false);
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

    fn is_word_boundary(&self, cursor: &mut H::Cursor, prev_char: Option<char>) -> bool {
        let is_word_char_before = if let Some(c) = prev_char {
            self.is_word_char(c)
        } else {
            false
        };

        let is_word_char_after = if let Some(c) = cursor.peek() {
            self.is_word_char(c)
        } else {
            false
        };

        is_word_char_before != is_word_char_after
    }

    fn is_word_char_at(&self, cursor: &mut H::Cursor) -> bool {
        if let Some(c) = cursor.peek() {
            self.is_word_char(c)
        } else {
            false
        }
    }

    fn is_word_char(&self, c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }
}

use crate::engine::{CompiledRegex, CompiledRegexHaystack, RegexEngine};
use crate::errors::CompileError;
use crate::parser::Parser;
// use std::sync::Arc; // Not needed if we clone explicitly

/// The backtracking regex engine.
#[derive(Clone, Copy, Debug, Default)]
pub struct BacktrackingRegexEngine;

impl RegexEngine for BacktrackingRegexEngine {
    type Regex = BacktrackingRegex;

    fn compile(&self, pattern: &str, flags: Flags) -> Result<Self::Regex, CompileError> {
        BacktrackingRegex::new(pattern, flags)
    }
}

/// A compiled regex using the backtracking engine.
#[derive(Clone, Debug)]
pub struct BacktrackingRegex {
    ast: Vec<AstNode>,
    flags: Flags,
    pattern: String,
}

impl BacktrackingRegex {
    /// Compiles a new backtracking regex.
    pub fn new(pattern: &str, mut flags: Flags) -> Result<Self, CompileError> {
        // Smartcase: if no explicit case flag, infer from pattern
        if flags.ignore_case.is_none() {
            let has_uppercase = pattern.chars().any(|c| c.is_uppercase());
            flags.ignore_case = Some(!has_uppercase);
        }

        let mut parser = Parser::new(pattern, flags);
        let ast = parser
            .parse()
            .map_err(|e| CompileError::InvalidPattern(e.to_string()))?;

        Ok(BacktrackingRegex {
            ast,
            flags,
            pattern: pattern.to_string(),
        })
    }
}

impl CompiledRegex for BacktrackingRegex {
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

    fn captures(&self, _text: &str) -> Option<crate::captures::Captures> {
        // TODO: Implement capture extraction in Matcher
        None
    }

    fn captures_all<'a>(
        &'a self,
        _text: &'a str,
    ) -> Box<dyn Iterator<Item = crate::captures::Captures> + 'a> {
        // TODO: Implement captures iterator
        Box::new(std::iter::empty())
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

impl crate::engine::CompiledRegexHaystack for BacktrackingRegex {
    fn is_match_from<H: Haystack>(&self, haystack: H) -> bool {
        self.find_from(haystack).is_some()
    }

    fn find_from<H: Haystack>(&self, haystack: H) -> Option<Match> {
        let matcher = Matcher::new(&self.ast, &self.flags, haystack);
        matcher.find()
    }

    fn find_from_at<H: Haystack>(&self, haystack: H, start: usize) -> Option<Match> {
        let matcher = Matcher::new(&self.ast, &self.flags, haystack);
        matcher.find_at(start)
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

// Iterator implementations for backtracker
struct FindMatchesIterator<'a, H: Haystack> {
    text: H,
    regex: &'a BacktrackingRegex,
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
