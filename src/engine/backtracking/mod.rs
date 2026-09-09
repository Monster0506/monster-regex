use crate::captures::Match;
use crate::flags::Flags;
use crate::parser::{AstNode, CharClass, SubroutineTarget};

use crate::haystack::{Haystack, HaystackCursor};
use std::cell::Cell;
use std::collections::HashMap;

const DEFAULT_MAX_BACKTRACK_STEPS: u64 = 2_000_000;

const MAX_SUBROUTINE_DEPTH: usize = 30;

type MatchCont<'c, C> =
    dyn FnMut(usize, &mut MatchContext, &mut C, Option<char>) -> Option<usize> + 'c;

/// The matching engine that walks the AST to find matches in text.
pub struct Matcher<'a, H: Haystack> {
    nodes: &'a [AstNode],
    flags: &'a Flags,
    text: H,
    prefilter: &'a Prefilter,
    steps: Cell<u64>,
    max_steps: u64,
    subroutine_groups: HashMap<usize, &'a AstNode>,
    /// Current live subroutine-call nesting depth - see
    /// `MAX_SUBROUTINE_DEPTH`.
    subroutine_depth: Cell<usize>,
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
    pub fn new(nodes: &'a [AstNode], flags: &'a Flags, text: H, prefilter: &'a Prefilter) -> Self {
        let mut subroutine_groups = HashMap::new();
        collect_subroutine_groups(nodes, &mut subroutine_groups);
        Self {
            nodes,
            flags,
            text,
            prefilter,
            steps: Cell::new(0),
            max_steps: flags
                .max_backtrack_steps
                .unwrap_or(DEFAULT_MAX_BACKTRACK_STEPS),
            subroutine_groups,
            subroutine_depth: Cell::new(0),
        }
    }

    #[inline]
    fn tick(&self) -> bool {
        let n = self.steps.get() + 1;
        self.steps.set(n);
        n <= self.max_steps
    }

    /// Finds the first match in the text.
    pub fn find(&self) -> Option<Match> {
        self.find_at(0)
    }

    /// Finds the first match in the text starting at the given position.
    pub fn find_at(&self, start_index: usize) -> Option<Match> {
        self.find_at_captures(start_index).map(|(m, _)| m)
    }

    pub fn find_at_captures(&self, start_index: usize) -> Option<(Match, Vec<Option<Match>>)> {
        // Determine max group index for context sizing
        let max_group = self.count_groups(self.nodes);
        let len = self.text.len();

        let mut context = MatchContext::new(max_group);

        let has_filter = self.prefilter.has_filter();
        let mut pos = start_index;

        while pos <= len {
            if has_filter {
                match self.prefilter.find_next(&self.text, pos) {
                    Some(p) => pos = p,
                    None => return None,
                }
                if pos > len {
                    break;
                }
            }

            let mut cursor = self.text.cursor_at(pos);
            let prev_char = if pos > 0 {
                self.text.char_before(pos)
            } else {
                None
            };

            context.clear();
            self.steps.set(0);
            let mut match_cursor = cursor.clone();

            if let Some(end_pos) = self.match_nodes(
                self.nodes,
                pos,
                &mut context,
                &mut match_cursor,
                prev_char,
                &mut |p, _, _, _| Some(p),
            ) {
                let start = context.match_start_override.unwrap_or(pos);
                let end = context.match_end_override.unwrap_or(end_pos);
                return Some((Match { start, end }, context.captures));
            }

            // No match at `pos`; advance one character and try again.
            if pos >= len {
                break;
            }
            match cursor.next() {
                Some(c) => pos += c.len_utf8(),
                None => break,
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
        prev_char: Option<char>,
        cont: &mut MatchCont<'_, H::Cursor>,
    ) -> Option<usize> {
        if !self.tick() {
            return None;
        }

        if nodes.is_empty() {
            return cont(pos, ctx, cursor, prev_char);
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
                    self.match_nodes(remaining, next_pos, ctx, cursor, Some(current_char), cont)
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
                    self.match_nodes(remaining, pos + len, ctx, cursor, Some(current_char), cont)
                } else {
                    None
                }
            }
            AstNode::StartAnchor => {
                let is_start = pos == 0;
                let is_line_start = self.flags.multiline && pos > 0 && prev_char == Some('\n');
                if is_start || is_line_start {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char, cont)
                } else {
                    None
                }
            }
            AstNode::EndAnchor => {
                let is_end = pos == self.text.len();
                let is_line_end =
                    self.flags.multiline && pos < self.text.len() && cursor.peek() == Some('\n');
                if is_end || is_line_end {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char, cont)
                } else {
                    None
                }
            }
            AstNode::WordBoundary => {
                if self.is_word_boundary(cursor, prev_char) {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char, cont)
                } else {
                    None
                }
            }
            AstNode::StartWord => {
                if self.is_word_boundary(cursor, prev_char) && self.is_word_char_at(cursor) {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char, cont)
                } else {
                    None
                }
            }
            AstNode::EndWord => {
                if self.is_word_boundary(cursor, prev_char) && !self.is_word_char_at(cursor) {
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char, cont)
                } else {
                    None
                }
            }
            AstNode::SetMatchStart => {
                let prev = ctx.match_start_override;
                ctx.match_start_override = Some(pos);
                let result = self.match_nodes(remaining, pos, ctx, cursor, prev_char, cont);
                if result.is_none() {
                    ctx.match_start_override = prev;
                }
                result
            }
            AstNode::SetMatchEnd => {
                let prev = ctx.match_end_override;
                ctx.match_end_override = Some(pos);
                let result = self.match_nodes(remaining, pos, ctx, cursor, prev_char, cont);
                if result.is_none() {
                    ctx.match_end_override = prev;
                }
                result
            }
            AstNode::Alternation(alts) => {
                for alt in alts {
                    let mut fork_cursor = cursor.clone();
                    let cont = &mut *cont;
                    if let Some(final_pos) = self.match_nodes(
                        alt,
                        pos,
                        ctx,
                        &mut fork_cursor,
                        prev_char,
                        &mut |next_pos, ctx, cursor, next_prev_char| {
                            self.match_nodes(remaining, next_pos, ctx, cursor, next_prev_char, cont)
                        },
                    ) {
                        *cursor = fork_cursor;
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
                let capture = *capture;
                let index = *index;
                self.match_nodes(
                    group_nodes,
                    pos,
                    ctx,
                    cursor,
                    prev_char,
                    &mut |next_pos, ctx, cursor, next_prev_char| {
                        let saved = if capture
                            && let Some(idx) = index
                            && idx < ctx.captures.len()
                        {
                            let prev = ctx.captures[idx].clone();
                            ctx.captures[idx] = Some(Match {
                                start: start_capture,
                                end: next_pos,
                            });
                            Some((idx, prev))
                        } else {
                            None
                        };
                        let result = self.match_nodes(
                            remaining,
                            next_pos,
                            ctx,
                            cursor,
                            next_prev_char,
                            cont,
                        );
                        if result.is_none()
                            && let Some((idx, prev)) = saved
                        {
                            ctx.captures[idx] = prev;
                        }
                        result
                    },
                )
            }
            AstNode::Subroutine(target) => {
                if self.subroutine_depth.get() >= MAX_SUBROUTINE_DEPTH {
                    return None;
                }
                let (call_nodes, capture, index): (&[AstNode], bool, Option<usize>) = match target
                {
                    SubroutineTarget::Whole => (self.nodes, false, None),
                    SubroutineTarget::Group(n) => match self.subroutine_groups.get(n) {
                        Some(AstNode::Group {
                            nodes,
                            capture,
                            index,
                            ..
                        }) => (nodes.as_slice(), *capture, *index),
                        _ => return None,
                    },
                    // Resolved to `Group` by the parser before matching
                    // ever runs - see `resolve_subroutines`.
                    SubroutineTarget::Name(_) => return None,
                };
                let start_capture = pos;
                self.subroutine_depth.set(self.subroutine_depth.get() + 1);
                let result = self.match_nodes(
                    call_nodes,
                    pos,
                    ctx,
                    cursor,
                    prev_char,
                    &mut |next_pos, ctx, cursor, next_prev_char| {
                        let saved = if capture
                            && let Some(idx) = index
                            && idx < ctx.captures.len()
                        {
                            let prev = ctx.captures[idx].clone();
                            ctx.captures[idx] = Some(Match {
                                start: start_capture,
                                end: next_pos,
                            });
                            Some((idx, prev))
                        } else {
                            None
                        };
                        let result = self.match_nodes(
                            remaining,
                            next_pos,
                            ctx,
                            cursor,
                            next_prev_char,
                            cont,
                        );
                        if result.is_none()
                            && let Some((idx, prev)) = saved
                        {
                            ctx.captures[idx] = prev;
                        }
                        result
                    },
                );
                self.subroutine_depth.set(self.subroutine_depth.get() - 1);
                result
            }
            AstNode::Backref(idx) => {
                if let Some(Some(m)) = ctx.captures.get(*idx) {
                    if self.text.matches_range(pos, m.start, m.end) {
                        let len_to_skip = m.end - m.start;
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
                        self.match_nodes(remaining, target_pos, ctx, cursor, last_char, cont)
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
                    .match_nodes(
                        look_nodes,
                        pos,
                        &mut look_ctx,
                        &mut look_cursor,
                        prev_char,
                        &mut |p, _, _, _| Some(p),
                    )
                    .is_some();
                if matched == *positive {
                    if *positive {
                        ctx.captures = look_ctx.captures;
                    }
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char, cont)
                } else {
                    None
                }
            }
            AstNode::LookBehind {
                nodes: look_nodes,
                positive,
            } => {
                // Try inner matches ending at `pos`, stepping back over whole chars
                // (boundary-safe) up to the lookbehind's max byte length.
                let max_len = max_consumable_bytes(look_nodes);
                let mut matched = false;
                let mut matched_captures = None;
                let mut start = pos;
                loop {
                    let mut look_ctx = ctx.clone();
                    let mut look_cursor = self.text.cursor_at(start);
                    let start_prev_char = if start > 0 {
                        self.text.char_before(start)
                    } else {
                        None
                    };

                    if self
                        .match_nodes(
                            look_nodes,
                            start,
                            &mut look_ctx,
                            &mut look_cursor,
                            start_prev_char,
                            &mut |p, _, _, _| if p == pos { Some(p) } else { None },
                        )
                        .is_some()
                    {
                        matched = true;
                        matched_captures = Some(look_ctx.captures);
                        break;
                    }

                    // Step to the previous char boundary, bounded by `max_len`.
                    if start == 0 {
                        break;
                    }
                    let prev = match self.text.char_before(start) {
                        Some(c) => c,
                        None => break,
                    };
                    let next_start = start - prev.len_utf8();
                    if max_len.is_some_and(|limit| pos - next_start > limit) {
                        break;
                    }
                    start = next_start;
                }

                if matched == *positive {
                    // Same reasoning as `LookAhead`: a successful *positive*
                    // lookbehind's captures stay visible afterward.
                    if *positive && let Some(captures) = matched_captures {
                        ctx.captures = captures;
                    }
                    self.match_nodes(remaining, pos, ctx, cursor, prev_char, cont)
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
                pos,
                ctx,
                cursor,
                prev_char,
                cont,
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
                pos,
                ctx,
                cursor,
                prev_char,
                cont,
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
                pos,
                ctx,
                cursor,
                prev_char,
                cont,
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
                pos,
                ctx,
                cursor,
                prev_char,
                cont,
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
                pos,
                ctx,
                cursor,
                prev_char,
                cont,
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
        origin: usize,
        ctx: &mut MatchContext,
        cursor: &mut H::Cursor,
        prev_char: Option<char>,
        cont: &mut MatchCont<'_, H::Cursor>,
    ) -> Option<usize> {
        if !self.tick() {
            return None;
        }

        if params.min > 0 {
            if !node_has_choice_point(node) {
                let ctx_snapshot = node_mutates_ctx(node).then(|| ctx.clone());
                let mut curr_pos = pos;
                let mut curr_prev = prev_char;
                for _ in 0..params.min {
                    match self.match_nodes(
                        std::slice::from_ref(node),
                        curr_pos,
                        ctx,
                        cursor,
                        curr_prev,
                        &mut |p, _, _, _| Some(p),
                    ) {
                        Some(next_pos) => {
                            if next_pos > curr_pos {
                                curr_prev = self.text.char_before(next_pos);
                            }
                            curr_pos = next_pos;
                        }
                        None => {
                            if let Some(snapshot) = ctx_snapshot {
                                *ctx = snapshot;
                            }
                            return None;
                        }
                    }
                }
                let result = self.match_quantifier_optional(
                    node,
                    params.max.map(|m| m - params.min),
                    params.greedy,
                    remaining,
                    curr_pos,
                    origin,
                    ctx,
                    cursor,
                    curr_prev,
                    cont,
                );
                if result.is_none()
                    && let Some(snapshot) = ctx_snapshot
                {
                    *ctx = snapshot;
                }
                return result;
            }

            return self.match_nodes(
                std::slice::from_ref(node),
                pos,
                ctx,
                cursor,
                prev_char,
                &mut |next_pos, ctx, cursor, next_prev_char| {
                    self.match_quantifier(
                        node,
                        QuantifierParams {
                            min: params.min - 1,
                            max: params.max.map(|m| m.saturating_sub(1)),
                            greedy: params.greedy,
                        },
                        remaining,
                        next_pos,
                        origin,
                        ctx,
                        cursor,
                        next_prev_char,
                        cont,
                    )
                },
            );
        }

        self.match_quantifier_optional(
            node,
            params.max,
            params.greedy,
            remaining,
            pos,
            origin,
            ctx,
            cursor,
            prev_char,
            cont,
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
        origin: usize,
        ctx: &mut MatchContext,
        cursor: &mut H::Cursor,
        prev_char: Option<char>,
        cont: &mut MatchCont<'_, H::Cursor>,
    ) -> Option<usize> {
        if !self.tick() {
            return None;
        }

        if let Some(0) = max_remaining {
            return self.match_nodes(remaining, pos, ctx, cursor, prev_char, cont);
        }

        let force_wider = node_is_alternation(node) && pos != origin;

        if greedy {
            let mut fork_cursor = cursor.clone();
            let took_more = {
                let cont = &mut *cont;
                self.match_nodes(
                    std::slice::from_ref(node),
                    pos,
                    ctx,
                    &mut fork_cursor,
                    prev_char,
                    &mut |next_pos, ctx, cursor, next_prev_char| {
                        if next_pos == pos {
                            if force_wider {
                                return None;
                            }
                            return self.match_nodes(
                                remaining,
                                next_pos,
                                ctx,
                                cursor,
                                next_prev_char,
                                cont,
                            );
                        }
                        self.match_quantifier_optional(
                            node,
                            max_remaining.map(|m| m - 1),
                            greedy,
                            remaining,
                            next_pos,
                            origin,
                            ctx,
                            cursor,
                            next_prev_char,
                            cont,
                        )
                    },
                )
            };
            if let Some(final_pos) = took_more {
                *cursor = fork_cursor;
                return Some(final_pos);
            }

            if force_wider {
                let mut fork_cursor = cursor.clone();
                let took_empty = {
                    let cont = &mut *cont;
                    self.match_nodes(
                        std::slice::from_ref(node),
                        pos,
                        ctx,
                        &mut fork_cursor,
                        prev_char,
                        &mut |next_pos, ctx, cursor, next_prev_char| {
                            self.match_nodes(remaining, next_pos, ctx, cursor, next_prev_char, cont)
                        },
                    )
                };
                if let Some(final_pos) = took_empty {
                    *cursor = fork_cursor;
                    return Some(final_pos);
                }
            }

            // Couldn't match one more, or everything after it failed - stop
            // here and try `remaining` directly, with clean state.
            self.match_nodes(remaining, pos, ctx, cursor, prev_char, cont)
        } else {
            let mut fork_cursor = cursor.clone();
            if let Some(final_pos) = {
                let cont = &mut *cont;
                self.match_nodes(remaining, pos, ctx, &mut fork_cursor, prev_char, cont)
            } {
                *cursor = fork_cursor;
                return Some(final_pos);
            }

            let mut fork_cursor = cursor.clone();
            let took_more = {
                let cont = &mut *cont;
                self.match_nodes(
                    std::slice::from_ref(node),
                    pos,
                    ctx,
                    &mut fork_cursor,
                    prev_char,
                    &mut |next_pos, ctx, cursor, next_prev_char| {
                        if next_pos == pos {
                            if force_wider {
                                return None;
                            }
                            return self.match_nodes(
                                remaining,
                                next_pos,
                                ctx,
                                cursor,
                                next_prev_char,
                                cont,
                            );
                        }
                        self.match_quantifier_optional(
                            node,
                            max_remaining.map(|m| m - 1),
                            greedy,
                            remaining,
                            next_pos,
                            origin,
                            ctx,
                            cursor,
                            next_prev_char,
                            cont,
                        )
                    },
                )
            };
            if let Some(final_pos) = took_more {
                *cursor = fork_cursor;
                return Some(final_pos);
            }

            if !force_wider {
                return None;
            }

            self.match_nodes(
                std::slice::from_ref(node),
                pos,
                ctx,
                cursor,
                prev_char,
                &mut |next_pos, ctx, cursor, next_prev_char| {
                    self.match_nodes(remaining, next_pos, ctx, cursor, next_prev_char, cont)
                },
            )
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
    prefilter: Prefilter,
    group_count: usize,
    named_groups: std::collections::HashMap<String, usize>,
}

fn analyze_captures(nodes: &[AstNode]) -> (usize, std::collections::HashMap<String, usize>) {
    fn visit(
        nodes: &[AstNode],
        count: &mut usize,
        map: &mut std::collections::HashMap<String, usize>,
    ) {
        for node in nodes {
            match node {
                AstNode::Group {
                    index,
                    nodes,
                    capture,
                    name,
                } => {
                    if *capture && let Some(i) = index {
                        *count = (*count).max(*i);
                        if let Some(n) = name {
                            map.insert(n.clone(), *i);
                        }
                    }
                    visit(nodes, count, map);
                }
                AstNode::Alternation(alts) => {
                    for alt in alts {
                        visit(alt, count, map);
                    }
                }
                AstNode::ZeroOrMore { node, .. }
                | AstNode::OneOrMore { node, .. }
                | AstNode::Optional { node, .. }
                | AstNode::Exact { node, .. }
                | AstNode::Range { node, .. } => {
                    visit(std::slice::from_ref(node), count, map);
                }
                AstNode::LookAhead { nodes, .. } | AstNode::LookBehind { nodes, .. } => {
                    visit(nodes, count, map);
                }
                _ => {}
            }
        }
    }
    let mut count = 0;
    let mut map = std::collections::HashMap::new();
    visit(nodes, &mut count, &mut map);
    (count, map)
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

        let prefilter = analyze_prefilter(&ast, &flags);
        let (group_count, named_groups) = analyze_captures(&ast);

        Ok(BacktrackingRegex {
            ast,
            flags,
            pattern: pattern.to_string(),
            prefilter,
            group_count,
            named_groups,
        })
    }

    fn build_captures(
        &self,
        full_match: Match,
        raw: Vec<Option<Match>>,
    ) -> crate::captures::Captures {
        let groups: Vec<Option<Match>> = (1..=self.group_count)
            .map(|i| raw.get(i).cloned().flatten())
            .collect();
        let named = self
            .named_groups
            .iter()
            .filter_map(|(name, &idx)| {
                groups
                    .get(idx - 1)
                    .cloned()
                    .flatten()
                    .map(|m| (name.clone(), m))
            })
            .collect();
        crate::captures::Captures {
            full_match,
            groups,
            named,
        }
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
            adjacent_empty: Default::default(),
        })
    }

    fn captures(&self, text: &str) -> Option<crate::captures::Captures> {
        let matcher = Matcher::new(&self.ast, &self.flags, text, &self.prefilter);
        let (full_match, raw) = matcher.find_at_captures(0)?;
        Some(self.build_captures(full_match, raw))
    }

    fn captures_all<'a>(
        &'a self,
        text: &'a str,
    ) -> Box<dyn Iterator<Item = crate::captures::Captures> + 'a> {
        Box::new(CapturesIterator {
            text,
            regex: self,
            last_end: 0,
            adjacent_empty: Default::default(),
        })
    }

    fn replace(&self, text: &str, replacement: &str) -> String {
        if let Some(caps) = self.captures(text) {
            let m = caps.full_match.clone();
            let mut result = String::with_capacity(text.len());
            result.push_str(&text[..m.start]);
            result.push_str(&crate::captures::expand_replacement(
                &caps,
                replacement,
                text,
            ));
            result.push_str(&text[m.end..]);
            result
        } else {
            text.to_string()
        }
    }

    fn replace_all(&self, text: &str, replacement: &str) -> String {
        let mut result = String::with_capacity(text.len() * 2);
        let mut last_end = 0;

        for caps in self.captures_all(text) {
            let m = caps.full_match.clone();
            result.push_str(&text[last_end..m.start]);
            result.push_str(&crate::captures::expand_replacement(
                &caps,
                replacement,
                text,
            ));
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
        let matcher = Matcher::new(&self.ast, &self.flags, haystack, &self.prefilter);
        matcher.find()
    }

    fn find_from_at<H: Haystack>(&self, haystack: H, start: usize) -> Option<Match> {
        let matcher = Matcher::new(&self.ast, &self.flags, haystack, &self.prefilter);
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
            adjacent_empty: Default::default(),
        })
    }
}

// Iterator implementations for backtracker
struct FindMatchesIterator<'a, H: Haystack> {
    text: H,
    regex: &'a BacktrackingRegex,
    last_end: usize,
    adjacent_empty: crate::captures::AdjacentEmptyFilter,
}

impl<'a, H: Haystack> Iterator for FindMatchesIterator<'a, H> {
    type Item = Match;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.last_end > self.text.len() {
                return None;
            }
            let m = self.regex.find_from_at(self.text, self.last_end)?;
            self.last_end = m.end.max(m.start + 1);
            if self.adjacent_empty.should_suppress(m.start, m.end) {
                continue;
            }
            return Some(m);
        }
    }
}

struct CapturesIterator<'a> {
    text: &'a str,
    regex: &'a BacktrackingRegex,
    last_end: usize,
    adjacent_empty: crate::captures::AdjacentEmptyFilter,
}

impl<'a> Iterator for CapturesIterator<'a> {
    type Item = crate::captures::Captures;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.last_end > self.text.len() {
                return None;
            }
            let matcher = Matcher::new(
                &self.regex.ast,
                &self.regex.flags,
                self.text,
                &self.regex.prefilter,
            );
            let (full_match, raw) = matcher.find_at_captures(self.last_end)?;
            self.last_end = full_match.end.max(full_match.start + 1);
            if self
                .adjacent_empty
                .should_suppress(full_match.start, full_match.end)
            {
                continue;
            }
            return Some(self.regex.build_captures(full_match, raw));
        }
    }
}

// -- Start prefilter ------------------------------------------------------------

/// A cheap test that locates the next position where a match could *start*,
/// letting `find_at` skip over input that provably cannot begin a match.
#[derive(Clone, Debug)]
pub enum Prefilter {
    /// No usable constraint - caller must try every position.
    None,
    /// The first consumed byte must equal this (case-sensitive).
    Byte(u8),
    /// The first consumed byte is one of a case pair (lowercase, uppercase).
    ByteCasePair(u8, u8),
    /// The match must begin with this ASCII literal run. When `ci` is set, `bytes`
    /// is stored lowercased and comparison is ASCII-case-insensitive.
    Literal { bytes: Box<[u8]>, ci: bool },
}

impl Prefilter {
    #[inline]
    fn has_filter(&self) -> bool {
        !matches!(self, Prefilter::None)
    }

    /// Find the next candidate match-start at or after `pos`, or `None` if no
    /// further candidate exists.
    #[inline]
    fn find_next<H: Haystack>(&self, text: &H, pos: usize) -> Option<usize> {
        match self {
            Prefilter::None => Some(pos),
            Prefilter::Byte(b) => text.find_byte(*b, pos),
            Prefilter::ByteCasePair(lo, up) => {
                if let Some(bytes) = text.as_bytes_opt() {
                    if pos >= bytes.len() {
                        return None;
                    }
                    memchr::memchr2(*lo, *up, &bytes[pos..]).map(|i| i + pos)
                } else {
                    min_opt(text.find_byte(*lo, pos), text.find_byte(*up, pos))
                }
            }
            Prefilter::Literal { bytes, ci } => {
                if let Some(hay) = text.as_bytes_opt() {
                    find_literal_bytes(hay, bytes, *ci, pos)
                } else {
                    let first = bytes[0];
                    if *ci {
                        let up = first.to_ascii_uppercase();
                        if first == up {
                            text.find_byte(first, pos)
                        } else {
                            min_opt(text.find_byte(first, pos), text.find_byte(up, pos))
                        }
                    } else {
                        text.find_byte(first, pos)
                    }
                }
            }
        }
    }
}

#[inline]
fn min_opt(a: Option<usize>, b: Option<usize>) -> Option<usize> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, y) => x.or(y),
    }
}

/// Locate `needle` within `hay[from..]`. When `ci` is set, `needle` is assumed to
/// already be lowercased and the comparison is ASCII-case-insensitive.
fn find_literal_bytes(hay: &[u8], needle: &[u8], ci: bool, from: usize) -> Option<usize> {
    if from > hay.len() {
        return None;
    }
    if needle.is_empty() {
        return Some(from);
    }
    if !ci {
        return memchr::memmem::find(&hay[from..], needle).map(|i| i + from);
    }

    // Case-insensitive ASCII: scan for the first byte (either case), then verify.
    let first = needle[0];
    let first_up = first.to_ascii_uppercase();
    let mut pos = from;
    loop {
        if pos >= hay.len() {
            return None;
        }
        let sub = &hay[pos..];
        let idx = if first != first_up {
            memchr::memchr2(first, first_up, sub)?
        } else {
            memchr::memchr(first, sub)?
        };
        let abs = pos + idx;
        let end = abs + needle.len();
        if end > hay.len() {
            return None;
        }
        if hay[abs..end]
            .iter()
            .zip(needle.iter())
            .all(|(&h, &n)| h.to_ascii_lowercase() == n)
        {
            return Some(abs);
        }
        pos = abs + 1;
    }
}

/// True for nodes that consume no input (they only assert a position).
fn is_zero_width(node: &AstNode) -> bool {
    matches!(
        node,
        AstNode::StartAnchor
            | AstNode::EndAnchor
            | AstNode::WordBoundary
            | AstNode::StartWord
            | AstNode::EndWord
            | AstNode::SetMatchStart
            | AstNode::SetMatchEnd
            | AstNode::LookAhead { .. }
            | AstNode::LookBehind { .. }
    )
}

/// Build a start prefilter from the pattern's leading nodes.
fn analyze_prefilter(nodes: &[AstNode], flags: &Flags) -> Prefilter {
    let ic = flags.ignore_case.unwrap_or(false);

    // Skip leading zero-width assertions.
    let mut i = 0;
    while i < nodes.len() && is_zero_width(&nodes[i]) {
        i += 1;
    }
    let nodes = &nodes[i..];
    if nodes.is_empty() {
        return Prefilter::None;
    }

    let mut run: Vec<u8> = Vec::new();
    for node in nodes {
        match node {
            AstNode::Literal(c) if c.is_ascii() => {
                run.push(if ic {
                    (*c as u8).to_ascii_lowercase()
                } else {
                    *c as u8
                });
            }
            _ => break,
        }
    }
    if run.len() >= 2 {
        return Prefilter::Literal {
            bytes: run.into_boxed_slice(),
            ci: ic,
        };
    }

    let first = match &nodes[0] {
        AstNode::Literal(c) => Some(*c),
        AstNode::OneOrMore { node, .. } => match &**node {
            AstNode::Literal(c) => Some(*c),
            _ => None,
        },
        AstNode::Exact { node, count } if *count > 0 => match &**node {
            AstNode::Literal(c) => Some(*c),
            _ => None,
        },
        _ => None,
    };

    match first {
        Some(c) if c.is_ascii() => {
            let b = c as u8;
            if ic {
                let lo = b.to_ascii_lowercase();
                let up = b.to_ascii_uppercase();
                if lo == up {
                    Prefilter::Byte(lo)
                } else {
                    Prefilter::ByteCasePair(lo, up)
                }
            } else {
                Prefilter::Byte(b)
            }
        }
        _ => Prefilter::None,
    }
}

fn collect_subroutine_groups<'a>(nodes: &'a [AstNode], map: &mut HashMap<usize, &'a AstNode>) {
    for node in nodes {
        match node {
            AstNode::Group {
                nodes: inner,
                index,
                ..
            } => {
                if let Some(i) = index {
                    map.insert(*i, node);
                }
                collect_subroutine_groups(inner, map);
            }
            AstNode::Alternation(alts) => {
                for alt in alts {
                    collect_subroutine_groups(alt, map);
                }
            }
            AstNode::ZeroOrMore { node, .. }
            | AstNode::OneOrMore { node, .. }
            | AstNode::Optional { node, .. }
            | AstNode::Exact { node, .. }
            | AstNode::Range { node, .. } => {
                collect_subroutine_groups(std::slice::from_ref(node.as_ref()), map);
            }
            AstNode::LookAhead { nodes, .. } | AstNode::LookBehind { nodes, .. } => {
                collect_subroutine_groups(nodes, map);
            }
            _ => {}
        }
    }
}

fn node_has_choice_point(node: &AstNode) -> bool {
    match node {
        AstNode::Alternation(_) => true,
        AstNode::Group { nodes, .. } => nodes.iter().any(node_has_choice_point),
        AstNode::ZeroOrMore { .. } | AstNode::OneOrMore { .. } | AstNode::Optional { .. } => true,
        AstNode::Exact { node, .. } => node_has_choice_point(node),
        AstNode::Range { node, min, max, .. } => max != &Some(*min) || node_has_choice_point(node),
        AstNode::Subroutine(_) => true,
        _ => false,
    }
}

fn node_is_alternation(node: &AstNode) -> bool {
    match node {
        AstNode::Alternation(_) => true,
        AstNode::Group { nodes, .. } if nodes.len() == 1 => node_is_alternation(&nodes[0]),
        _ => false,
    }
}

fn node_mutates_ctx(node: &AstNode) -> bool {
    match node {
        AstNode::Group {
            capture: true,
            index: Some(_),
            ..
        } => true,
        AstNode::Group { nodes, .. } => nodes.iter().any(node_mutates_ctx),
        AstNode::SetMatchStart | AstNode::SetMatchEnd => true,
        AstNode::Alternation(alts) => alts.iter().any(|alt| alt.iter().any(node_mutates_ctx)),
        AstNode::ZeroOrMore { node, .. }
        | AstNode::OneOrMore { node, .. }
        | AstNode::Optional { node, .. }
        | AstNode::Exact { node, .. }
        | AstNode::Range { node, .. } => node_mutates_ctx(node),
        AstNode::Subroutine(_) => true,
        _ => false,
    }
}

fn max_consumable_bytes(nodes: &[AstNode]) -> Option<usize> {
    let mut total = 0usize;
    for n in nodes {
        total = total.checked_add(node_max_consumable_bytes(n)?)?;
    }
    Some(total)
}

fn node_max_consumable_bytes(node: &AstNode) -> Option<usize> {
    match node {
        AstNode::Literal(c) => Some(c.len_utf8()),
        // A character class matches exactly one code point: at most 4 UTF-8 bytes.
        AstNode::CharClass(_) => Some(4),
        // Zero-width assertions consume no input.
        AstNode::StartAnchor
        | AstNode::EndAnchor
        | AstNode::WordBoundary
        | AstNode::StartWord
        | AstNode::EndWord
        | AstNode::SetMatchStart
        | AstNode::SetMatchEnd
        | AstNode::LookAhead { .. }
        | AstNode::LookBehind { .. } => Some(0),
        AstNode::Optional { node, .. } => node_max_consumable_bytes(node),
        AstNode::ZeroOrMore { .. } | AstNode::OneOrMore { .. } => None,
        AstNode::Exact { node, count } => node_max_consumable_bytes(node)?.checked_mul(*count),
        AstNode::Range { node, max, .. } => match max {
            Some(m) => node_max_consumable_bytes(node)?.checked_mul(*m),
            None => None,
        },
        AstNode::Group { nodes, .. } => max_consumable_bytes(nodes),
        AstNode::Alternation(alts) => {
            let mut best = 0usize;
            for alt in alts {
                best = best.max(max_consumable_bytes(alt)?);
            }
            Some(best)
        }
        AstNode::Backref(_) | AstNode::Subroutine(_) => None,
    }
}
