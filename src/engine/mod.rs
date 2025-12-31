use crate::captures::Match;
use crate::flags::Flags;
use crate::parser::{AstNode, CharClass};

/// The matching engine that walks the AST to find matches in text.
pub struct Matcher<'a> {
    nodes: &'a [AstNode],
    flags: &'a Flags,
    text: &'a str,
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
}

impl<'a> Matcher<'a> {
    /// Creates a new Matcher instance.
    pub fn new(nodes: &'a [AstNode], flags: &'a Flags, text: &'a str) -> Self {
        Self { nodes, flags, text }
    }

    /// Finds the first match in the text.
    pub fn find(&self) -> Option<Match> {
        // Determine max group index for context sizing
        let max_group = self.count_groups(self.nodes);

        // Try to match starting at every character boundary
        for (start_pos, _) in self.text.char_indices() {
            let mut context = MatchContext::new(max_group);
            if let Some(end_pos) = self.match_nodes(self.nodes, start_pos, &mut context) {
                let start = context.match_start_override.unwrap_or(start_pos);
                let end = context.match_end_override.unwrap_or(end_pos);
                return Some(Match { start, end });
            }
        }

        // Also try matching at the very end of the string (for empty matches or anchors)
        let mut context = MatchContext::new(max_group);
        if let Some(end_pos) = self.match_nodes(self.nodes, self.text.len(), &mut context) {
            let start = context.match_start_override.unwrap_or(self.text.len());
            let end = context.match_end_override.unwrap_or(end_pos);
            return Some(Match { start, end });
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

    fn match_nodes(&self, nodes: &[AstNode], pos: usize, ctx: &mut MatchContext) -> Option<usize> {
        if nodes.is_empty() {
            return Some(pos);
        }

        let node = &nodes[0];
        let remaining = &nodes[1..];

        match node {
            AstNode::Literal(c) => {
                let char_len = c.len_utf8();
                if pos + char_len > self.text.len() {
                    return None;
                }

                let matches = if self.flags.ignore_case.unwrap_or(false) {
                    let current_char = self.text[pos..].chars().next()?;
                    c.to_lowercase().eq(current_char.to_lowercase())
                } else {
                    self.text[pos..].starts_with(*c)
                };

                if matches {
                    let next_pos = pos + self.text[pos..].chars().next()?.len_utf8();
                    self.match_nodes(remaining, next_pos, ctx)
                } else {
                    None
                }
            }
            AstNode::CharClass(class) => {
                let current_char = self.text[pos..].chars().next()?;
                if self.match_char_class(class, current_char) {
                    self.match_nodes(remaining, pos + current_char.len_utf8(), ctx)
                } else {
                    None
                }
            }
            AstNode::StartAnchor => {
                let is_start = pos == 0;
                let is_line_start =
                    self.flags.multiline && pos > 0 && self.text.as_bytes()[pos - 1] == b'\n';
                if is_start || is_line_start {
                    self.match_nodes(remaining, pos, ctx)
                } else {
                    None
                }
            }
            AstNode::EndAnchor => {
                let is_end = pos == self.text.len();
                let is_line_end = self.flags.multiline
                    && pos < self.text.len()
                    && self.text.as_bytes()[pos] == b'\n';
                if is_end || is_line_end {
                    self.match_nodes(remaining, pos, ctx)
                } else {
                    None
                }
            }
            AstNode::WordBoundary => {
                if self.is_word_boundary(pos) {
                    self.match_nodes(remaining, pos, ctx)
                } else {
                    None
                }
            }
            AstNode::StartWord => {
                if self.is_word_boundary(pos) && self.is_word_char_at(pos) {
                    self.match_nodes(remaining, pos, ctx)
                } else {
                    None
                }
            }
            AstNode::EndWord => {
                if self.is_word_boundary(pos) && !self.is_word_char_at(pos) {
                    self.match_nodes(remaining, pos, ctx)
                } else {
                    None
                }
            }
            AstNode::SetMatchStart => {
                ctx.match_start_override = Some(pos);
                self.match_nodes(remaining, pos, ctx)
            }
            AstNode::SetMatchEnd => {
                ctx.match_end_override = Some(pos);
                self.match_nodes(remaining, pos, ctx)
            }
            AstNode::Alternation(alts) => {
                for alt in alts {
                    // Snapshot context
                    let mut fork_ctx = ctx.clone();
                    if let Some(next_pos) = self.match_nodes(alt, pos, &mut fork_ctx)
                        && let Some(final_pos) =
                            self.match_nodes(remaining, next_pos, &mut fork_ctx)
                    {
                        *ctx = fork_ctx;
                        return Some(final_pos);
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
                if let Some(next_pos) = self.match_nodes(group_nodes, pos, ctx)
                    && *capture
                    && let Some(idx) = index
                    && *idx < ctx.captures.len()
                {
                    ctx.captures[*idx] = Some(Match {
                        start: start_capture,
                        end: next_pos,
                    });

                    self.match_nodes(remaining, next_pos, ctx)
                } else {
                    None
                }
            }
            AstNode::Backref(idx) => {
                if let Some(Some(m)) = ctx.captures.get(*idx) {
                    let captured_text = &self.text[m.start..m.end];
                    if self.text[pos..].starts_with(captured_text) {
                        self.match_nodes(remaining, pos + captured_text.len(), ctx)
                    } else {
                        None
                    }
                } else {
                    // Backref to non-existent group fails
                    None
                }
            }
            AstNode::LookAhead {
                nodes: look_nodes,
                positive,
            } => {
                let mut look_ctx = ctx.clone();
                let matched = self.match_nodes(look_nodes, pos, &mut look_ctx).is_some();
                if matched == *positive {
                    self.match_nodes(remaining, pos, ctx)
                } else {
                    None
                }
            }
            AstNode::LookBehind {
                nodes: look_nodes,
                positive,
            } => {
                // Lookbehind implementation: try matching ending at pos
                let mut matched = false;
                for start in 0..=pos {
                    let mut look_ctx = ctx.clone();
                    if let Some(end) = self.match_nodes(look_nodes, start, &mut look_ctx)
                        && end == pos
                    {
                        matched = true;
                        break;
                    }
                }

                if matched == *positive {
                    self.match_nodes(remaining, pos, ctx)
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
            ),
        }
    }

    fn match_quantifier(
        &self,
        node: &AstNode,
        params: QuantifierParams,
        remaining: &[AstNode],
        pos: usize,
        ctx: &mut MatchContext,
    ) -> Option<usize> {
        // 1. Match minimum required times
        let mut curr_pos = pos;
        for _ in 0..params.min {
            if let Some(next_pos) = self.match_nodes(std::slice::from_ref(node), curr_pos, ctx) {
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
        )
    }

    fn match_quantifier_optional(
        &self,
        node: &AstNode,
        max_remaining: Option<usize>,
        greedy: bool,
        remaining: &[AstNode],
        pos: usize,
        ctx: &mut MatchContext,
    ) -> Option<usize> {
        if let Some(0) = max_remaining {
            return self.match_nodes(remaining, pos, ctx);
        }

        if greedy {
            // Try to match one more
            let mut fork_ctx = ctx.clone();
            if let Some(next_pos) = self.match_nodes(std::slice::from_ref(node), pos, &mut fork_ctx)
            {
                // Prevent infinite loops on zero-width matches
                if next_pos > pos
                    && let Some(final_pos) = self.match_quantifier_optional(
                        node,
                        max_remaining.map(|m| m - 1),
                        greedy,
                        remaining,
                        next_pos,
                        &mut fork_ctx,
                    )
                {
                    *ctx = fork_ctx;
                    return Some(final_pos);
                }
            }

            // If we couldn't match more, or the recursive call failed, try matching the rest
            self.match_nodes(remaining, pos, ctx)
        } else {
            // Lazy: Try matching the rest first
            let mut fork_ctx = ctx.clone();
            if let Some(final_pos) = self.match_nodes(remaining, pos, &mut fork_ctx) {
                *ctx = fork_ctx;
                return Some(final_pos);
            }

            // If that fails, try matching one more
            if let Some(next_pos) = self.match_nodes(std::slice::from_ref(node), pos, ctx)
                && next_pos > pos
            {
                return self.match_quantifier_optional(
                    node,
                    max_remaining.map(|m| m - 1),
                    greedy,
                    remaining,
                    next_pos,
                    ctx,
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
            CharClass::Lowercase => c.is_lowercase(),
            CharClass::NonLowercase => !c.is_lowercase(),
            CharClass::Uppercase => c.is_uppercase(),
            CharClass::NonUppercase => !c.is_uppercase(),
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
                let found = chars.iter().any(|range| c >= range.start && c <= range.end);
                if *negated { !found } else { found }
            }
        }
    }

    fn is_word_boundary(&self, pos: usize) -> bool {
        let is_word_char_before = if pos > 0 {
            self.text[..pos]
                .chars()
                .last()
                .is_some_and(|c| self.is_word_char(c))
        } else {
            false
        };

        let is_word_char_after = if pos < self.text.len() {
            self.text[pos..]
                .chars()
                .next()
                .is_some_and(|c| self.is_word_char(c))
        } else {
            false
        };

        is_word_char_before != is_word_char_after
    }

    fn is_word_char_at(&self, pos: usize) -> bool {
        if pos < self.text.len() {
            self.text[pos..]
                .chars()
                .next()
                .is_some_and(|c| self.is_word_char(c))
        } else {
            false
        }
    }

    fn is_word_char(&self, c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }
}
