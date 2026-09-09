use crate::flags::Flags;
use std::collections::HashMap;
use std::fmt;

/// Represents a node in the Abstract Syntax Tree (AST) of a regular expression.
#[derive(Debug, Clone, PartialEq)]
pub enum AstNode {
    /// A literal character match.
    Literal(char),

    /// A character class (e.g., `\d`, `[a-z]`, `.`).
    CharClass(CharClass),

    /// Start of string (or line in multiline mode) anchor `^`.
    StartAnchor,
    /// End of string (or line in multiline mode) anchor `$`.
    EndAnchor,
    /// Word boundary anchor `\b`.
    WordBoundary,
    /// Start of word anchor `\<`.
    StartWord,
    /// End of word anchor `\>`.
    EndWord,
    /// Sets the start of the match `\zs`.
    SetMatchStart,
    /// Sets the end of the match `\ze`.
    SetMatchEnd,

    /// Zero or more repetitions `*`.
    ZeroOrMore {
        /// The node being repeated.
        node: Box<AstNode>,
        /// Whether the quantifier is greedy (default) or lazy (`?`).
        greedy: bool,
    },
    /// One or more repetitions `+`.
    OneOrMore {
        /// The node being repeated.
        node: Box<AstNode>,
        /// Whether the quantifier is greedy (default) or lazy (`?`).
        greedy: bool,
    },
    /// Zero or one repetition `?`.
    Optional {
        /// The node being repeated.
        node: Box<AstNode>,
        /// Whether the quantifier is greedy (default) or lazy (`?`).
        greedy: bool,
    },
    /// Exact number of repetitions `{n}`.
    Exact {
        /// The node being repeated.
        node: Box<AstNode>,
        /// The exact count.
        count: usize,
    },
    /// Range of repetitions `{n,m}` or `{n,}`.
    Range {
        /// The node being repeated.
        node: Box<AstNode>,
        /// The minimum count.
        min: usize,
        /// The maximum count (None means infinite).
        max: Option<usize>,
        /// Whether the quantifier is greedy (default) or lazy (`?`).
        greedy: bool,
    },

    /// A capturing or non-capturing group `(...)`.
    Group {
        /// The sequence of nodes inside the group.
        nodes: Vec<AstNode>,
        /// The name of the group, if it is a named capture `(?<name>...)`.
        name: Option<String>,
        /// Whether this group captures text.
        capture: bool,
        /// The index of the capture group (1-based), if capturing.
        index: Option<usize>,
    },
    /// Alternation `|`.
    Alternation(Vec<Vec<AstNode>>),

    /// Backreference to a captured group `\n`.
    Backref(usize),

    /// Lookahead assertion `(?>=...)` or `(?>!...)`.
    LookAhead {
        /// The sequence of nodes to check ahead.
        nodes: Vec<AstNode>,
        /// True for positive lookahead, false for negative.
        positive: bool,
    },
    /// Lookbehind assertion `(?<=...)` or `(?<!...)`.
    LookBehind {
        /// The sequence of nodes to check behind.
        nodes: Vec<AstNode>,
        /// True for positive lookbehind, false for negative.
        positive: bool,
    },

    Subroutine(SubroutineTarget),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SubroutineTarget {
    /// `(?R)` - recurse into the whole pattern.
    Whole,
    /// `(?N)`, or a `(?-N)`/`(?+N)` already resolved to an absolute number.
    Group(usize),
    /// `(?&name)` / `(?P>name)`, not yet resolved to a group number.
    Name(String),
}

/// Represents a class of characters.
#[derive(Debug, Clone, PartialEq)]
pub enum CharClass {
    // Standard classes
    /// Digit `\d` (`[0-9]`).
    Digit,
    /// Non-digit `\D`.
    NonDigit,
    /// Word character `\w` (`[a-zA-Z0-9_]`).
    Word,
    /// Non-word character `\W`.
    NonWord,
    /// Whitespace `\s` (`[ \t\r\n\f\v]`).
    Whitespace,
    /// Non-whitespace `\S`.
    NonWhitespace,

    // Extended classes
    /// Lowercase character `\l`.
    Lowercase,
    /// Non-lowercase character `\L`.
    NonLowercase,
    /// Uppercase character `\u`.
    Uppercase,
    /// Non-uppercase character `\U`.
    NonUppercase,
    /// Hexadecimal digit `\x`.
    Hex,
    /// Non-hexadecimal digit `\X`.
    NonHex,
    /// Octal digit `\o`.
    Octal,
    /// Non-octal digit `\O`.
    NonOctal,
    /// Start of word character `\h`.
    WordStart,
    /// Non-start of word character `\H`.
    NonWordStart,
    /// Punctuation `\p`.
    Punctuation,
    /// Non-punctuation `\P`.
    NonPunctuation,
    /// Alphanumeric `\a`.
    Alphanumeric,
    /// Non-alphanumeric `\A`.
    NonAlphanumeric,

    // Custom sets
    /// Custom character set `[...]`.
    Set {
        /// The ranges or characters included in the set.
        chars: Vec<CharRange>,
        /// Whether the set is negated `[^...]`.
        negated: bool,
    },

    /// Dot `.` (matches any character except newline, or any character with `s` flag).
    Dot,
}

/// A range of characters in a character set.
#[derive(Debug, Clone, PartialEq)]
pub struct CharRange {
    /// Start of the range.
    pub start: char,
    /// End of the range.
    pub end: char,
}

const MAX_GROUP_DEPTH: usize = 200;

/// The recursive descent parser for the regex pattern.
#[derive(Debug, Clone)]
pub struct Parser {
    input: Vec<char>,
    pos: usize,
    flags: Flags,
    group_count: usize,
    depth: usize,
}

/// Errors that can occur during parsing.
#[derive(Debug)]
pub enum ParseError {
    UnexpectedChar(char, usize),
    UnexpectedEof,
    InvalidQuantifier(String),
    UnmatchedParen,
    InvalidGroupName(String),
    InvalidEscape(char),
    InvalidCharClass,
    DuplicateGroupName(String),
    InvalidBackref(usize),
    InvalidLineNumber(String),
    InvalidGroup(String),
    /// A pattern nested groups/lookarounds more than `MAX_GROUP_DEPTH` deep.
    NestingTooDeep(usize),
    InvalidSubroutineReference(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::UnexpectedChar(c, pos) => {
                write!(f, "Unexpected '{}' at position {}", c, pos)
            }
            ParseError::UnexpectedEof => write!(f, "Unexpected end of input"),
            ParseError::InvalidQuantifier(s) => {
                write!(f, "Invalid quantifier: {}", s)
            }
            ParseError::UnmatchedParen => write!(f, "Unmatched parenthesis"),
            ParseError::InvalidGroupName(s) => {
                write!(f, "Invalid group name: {}", s)
            }
            ParseError::InvalidEscape(c) => {
                write!(f, "Invalid escape sequence: \\{}", c)
            }
            ParseError::InvalidCharClass => {
                write!(f, "Invalid character class")
            }
            ParseError::DuplicateGroupName(s) => {
                write!(f, "Duplicate group name: {}", s)
            }
            ParseError::InvalidBackref(n) => {
                write!(f, "Invalid backreference: \\{}", n)
            }
            ParseError::InvalidLineNumber(s) => {
                write!(f, "Invalid line number: {}", s)
            }
            ParseError::InvalidGroup(s) => {
                write!(f, "Invalid group syntax: {}", s)
            }
            ParseError::NestingTooDeep(max) => {
                write!(f, "Pattern nests groups more than {} levels deep", max)
            }
            ParseError::InvalidSubroutineReference(s) => {
                write!(f, "Invalid subroutine reference: {}", s)
            }
        }
    }
}

impl std::error::Error for ParseError {}

impl Parser {
    /// Creates a new parser for the given pattern.
    pub fn new(pattern: &str, flags: Flags) -> Self {
        Parser {
            input: pattern.chars().collect(),
            pos: 0,
            flags,
            group_count: 0,
            depth: 0,
        }
    }

    /// Parses the pattern into an AST.
    pub fn parse(&mut self) -> Result<Vec<AstNode>, ParseError> {
        let mut nodes = self.parse_alternation()?;
        let mut names = HashMap::new();
        collect_group_names(&nodes, &mut names);
        resolve_subroutines(&mut nodes, &names, self.group_count)?;
        Ok(nodes)
    }

    // Top level: handle |
    fn parse_alternation(&mut self) -> Result<Vec<AstNode>, ParseError> {
        let mut alternatives = vec![];
        let mut current = self.parse_sequence()?;

        while self.peek() == Some('|') {
            self.consume()?;
            alternatives.push(current);
            current = self.parse_sequence()?;
        }
        alternatives.push(current);

        if alternatives.len() == 1 {
            Ok(alternatives.pop().unwrap())
        } else {
            Ok(vec![AstNode::Alternation(alternatives)])
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        if !self.flags.verbose {
            return;
        }
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            if ch.is_whitespace() {
                self.pos += 1;
            } else if ch == '#' {
                self.pos += 1;
                while self.pos < self.input.len() && self.input[self.pos] != '\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    // Parse sequence of atoms with quantifiers
    fn parse_sequence(&mut self) -> Result<Vec<AstNode>, ParseError> {
        let mut nodes = vec![];

        loop {
            self.skip_whitespace_and_comments();
            match self.current() {
                Some(&'|') | Some(&')') | None => break,
                _ => {
                    let node = self.parse_atom()?;
                    let node = self.apply_quantifier(node)?;
                    nodes.push(node);
                }
            }
        }

        Ok(nodes)
    }

    // Parse a single atom (before quantifiers)
    fn parse_atom(&mut self) -> Result<AstNode, ParseError> {
        match self.current() {
            None => Err(ParseError::UnexpectedEof),
            Some(&'.') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::Dot))
            }
            Some(&'^') => {
                self.consume()?;
                Ok(AstNode::StartAnchor)
            }
            Some(&'$') => {
                self.consume()?;
                Ok(AstNode::EndAnchor)
            }
            Some(&'[') => self.parse_char_class(),
            Some(&'(') => self.parse_group(),
            Some(&'\\') => self.parse_escape(),
            Some(&ch) => {
                self.consume()?;
                Ok(AstNode::Literal(ch))
            }
        }
    }

    // Parse \escape sequences
    fn parse_escape(&mut self) -> Result<AstNode, ParseError> {
        self.consume()?; // consume \

        match self.current() {
            None => Err(ParseError::UnexpectedEof),
            Some(&'d') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::Digit))
            }
            Some(&'D') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::NonDigit))
            }
            Some(&'w') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::Word))
            }
            Some(&'W') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::NonWord))
            }
            Some(&'s') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::Whitespace))
            }
            Some(&'S') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::NonWhitespace))
            }
            Some(&'l') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::Lowercase))
            }
            Some(&'L') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::NonLowercase))
            }
            Some(&'u') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::Uppercase))
            }
            Some(&'U') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::NonUppercase))
            }
            Some(&'x') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::Hex))
            }
            Some(&'X') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::NonHex))
            }
            Some(&'o') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::Octal))
            }
            Some(&'O') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::NonOctal))
            }
            Some(&'h') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::WordStart))
            }
            Some(&'H') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::NonWordStart))
            }
            Some(&'p') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::Punctuation))
            }
            Some(&'P') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::NonPunctuation))
            }
            Some(&'a') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::Alphanumeric))
            }
            Some(&'A') => {
                self.consume()?;
                Ok(AstNode::CharClass(CharClass::NonAlphanumeric))
            }
            Some(&'b') => {
                self.consume()?;
                Ok(AstNode::WordBoundary)
            }
            Some(&'<') => {
                self.consume()?;
                Ok(AstNode::StartWord)
            }
            Some(&'>') => {
                self.consume()?;
                Ok(AstNode::EndWord)
            }
            Some(&'z') => {
                self.consume()?;
                match self.current() {
                    Some(&'s') => {
                        self.consume()?;
                        Ok(AstNode::SetMatchStart)
                    }
                    Some(&'e') => {
                        self.consume()?;
                        Ok(AstNode::SetMatchEnd)
                    }
                    _ => Err(ParseError::InvalidEscape('z')),
                }
            }
            Some(&c @ '0'..='9') => {
                self.consume()?;
                let digit = c.to_digit(10).unwrap() as usize;
                Ok(AstNode::Backref(digit))
            }
            Some(&'n') => {
                self.consume()?;
                Ok(AstNode::Literal('\n'))
            }
            Some(&'t') => {
                self.consume()?;
                Ok(AstNode::Literal('\t'))
            }
            Some(&'r') => {
                self.consume()?;
                Ok(AstNode::Literal('\r'))
            }
            Some(&'f') => {
                self.consume()?;
                Ok(AstNode::Literal('\x0C'))
            }
            Some(&'v') => {
                self.consume()?;
                Ok(AstNode::Literal('\x0B'))
            }
            Some(&'\\') => {
                self.consume()?;
                Ok(AstNode::Literal('\\'))
            }
            Some(&ch) => {
                self.consume()?;
                // Literal escape (e.g. \*, \[)
                Ok(AstNode::Literal(ch))
            }
        }
    }

    // Parse (group) or (?:non-capture) or (?<name>) or lookarounds
    fn parse_group(&mut self) -> Result<AstNode, ParseError> {
        self.consume()?; // consume (

        self.depth += 1;
        if self.depth > MAX_GROUP_DEPTH {
            return Err(ParseError::NestingTooDeep(MAX_GROUP_DEPTH));
        }

        let result = if self.current() == Some(&'?') {
            self.consume()?;
            self.parse_extended_group()
        } else {
            // Capturing group
            self.group_count += 1;
            let index = self.group_count;
            self.parse_alternation().and_then(|nodes| {
                self.expect_close_paren()?;
                Ok(AstNode::Group {
                    nodes,
                    name: None,
                    capture: true,
                    index: Some(index),
                })
            })
        };

        self.depth -= 1;
        result
    }

    fn parse_extended_group(&mut self) -> Result<AstNode, ParseError> {
        match self.current() {
            Some(&':') => {
                self.consume()?;
                let nodes = self.parse_alternation()?;
                self.expect_close_paren()?;
                Ok(AstNode::Group {
                    nodes,
                    name: None,
                    capture: false,
                    index: None,
                })
            }
            Some(&'<') => {
                self.consume()?;
                // Check for lookbehind
                match self.current() {
                    Some(&'=') => {
                        self.consume()?;
                        let nodes = self.parse_alternation()?;
                        self.expect_close_paren()?;
                        Ok(AstNode::LookBehind {
                            nodes,
                            positive: true,
                        })
                    }
                    Some(&'!') => {
                        self.consume()?;
                        let nodes = self.parse_alternation()?;
                        self.expect_close_paren()?;
                        Ok(AstNode::LookBehind {
                            nodes,
                            positive: false,
                        })
                    }
                    _ => {
                        // Named capture (?<name>...)
                        let name = self.parse_group_name()?;
                        if self.current() != Some(&'>') {
                            return Err(ParseError::InvalidGroupName("expected '>'".to_string()));
                        }
                        self.consume()?;

                        self.group_count += 1;
                        let index = self.group_count;

                        let nodes = self.parse_alternation()?;
                        self.expect_close_paren()?;
                        Ok(AstNode::Group {
                            nodes,
                            name: Some(name),
                            capture: true,
                            index: Some(index),
                        })
                    }
                }
            }
            Some(&'>') => {
                self.consume()?;
                match self.current() {
                    Some(&'=') => {
                        self.consume()?;
                        let nodes = self.parse_alternation()?;
                        self.expect_close_paren()?;
                        Ok(AstNode::LookAhead {
                            nodes,
                            positive: true,
                        })
                    }
                    Some(&'!') => {
                        self.consume()?;
                        let nodes = self.parse_alternation()?;
                        self.expect_close_paren()?;
                        Ok(AstNode::LookAhead {
                            nodes,
                            positive: false,
                        })
                    }
                    _ => Err(ParseError::InvalidGroup(
                        "Expected = or ! after ?>".to_string(),
                    )),
                }
            }
            // Subroutine call: recurse the whole pattern `(?R)`.
            Some(&'R') => {
                self.consume()?;
                self.expect_close_paren()?;
                Ok(AstNode::Subroutine(SubroutineTarget::Whole))
            }
            // Subroutine call: named group `(?&name)`. Resolved to a group
            // number in a post-parse pass - see `resolve_subroutines`.
            Some(&'&') => {
                self.consume()?;
                let name = self.parse_group_name()?;
                self.expect_close_paren()?;
                Ok(AstNode::Subroutine(SubroutineTarget::Name(name)))
            }
            // Subroutine call, alternate named syntax: `(?P>name)`.
            Some(&'P') => {
                self.consume()?;
                if self.current() != Some(&'>') {
                    return Err(ParseError::InvalidGroup(
                        "Expected '>' after ?P".to_string(),
                    ));
                }
                self.consume()?;
                let name = self.parse_group_name()?;
                self.expect_close_paren()?;
                Ok(AstNode::Subroutine(SubroutineTarget::Name(name)))
            }
            // Subroutine call: absolute group number `(?N)`.
            Some(&c) if c.is_ascii_digit() => {
                let n = self.parse_number()?;
                self.expect_close_paren()?;
                Ok(AstNode::Subroutine(SubroutineTarget::Group(n)))
            }
            Some(&sign @ ('-' | '+')) => {
                self.consume()?;
                let n = self.parse_number()?;
                self.expect_close_paren()?;
                let count = self.group_count as isize;
                let target = if sign == '-' {
                    count - (n as isize) + 1
                } else {
                    count + n as isize
                };
                if target < 1 {
                    return Err(ParseError::InvalidSubroutineReference(format!(
                        "(?{sign}{n}) has no target group"
                    )));
                }
                Ok(AstNode::Subroutine(SubroutineTarget::Group(
                    target as usize,
                )))
            }
            _ => Err(ParseError::InvalidGroup("Unknown extension ?".to_string())),
        }
    }

    // Parse group name [a-zA-Z_][a-zA-Z0-9_]*
    fn parse_group_name(&mut self) -> Result<String, ParseError> {
        let mut name = String::new();

        loop {
            match self.current() {
                Some(&c) if c.is_alphanumeric() || c == '_' => {
                    name.push(c);
                    self.consume()?;
                }
                _ => break,
            }
        }

        if name.is_empty() {
            return Err(ParseError::InvalidGroupName("empty name".to_string()));
        }

        Ok(name)
    }

    // Parse [char class]
    fn parse_char_class(&mut self) -> Result<AstNode, ParseError> {
        self.consume()?; // consume [

        let negated = if self.current() == Some(&'^') {
            self.consume()?;
            true
        } else {
            false
        };

        let mut ranges = vec![];

        loop {
            match self.current() {
                None => return Err(ParseError::UnexpectedEof),
                Some(&']') => {
                    self.consume()?;
                    break;
                }
                Some(&'\\') => {
                    // Escaped char in class
                    self.consume()?;
                    match self.current() {
                        Some(&c) => {
                            self.consume()?;
                            ranges.push(CharRange { start: c, end: c });
                        }
                        None => return Err(ParseError::UnexpectedEof),
                    }
                }
                Some(&c) => {
                    self.consume()?;
                    // Check for range
                    if self.current() == Some(&'-')
                        && self.peek_ahead(1).is_some()
                        && self.peek_ahead(1) != Some(&']')
                    {
                        self.consume()?;
                        match self.current() {
                            Some(&end) => {
                                self.consume()?;
                                ranges.push(CharRange { start: c, end });
                            }
                            None => return Err(ParseError::UnexpectedEof),
                        }
                    } else {
                        ranges.push(CharRange { start: c, end: c });
                    }
                }
            }
        }

        Ok(AstNode::CharClass(CharClass::Set {
            chars: ranges,
            negated,
        }))
    }

    // Apply quantifiers: *, +, ?, {n}, {n,m}, etc
    fn apply_quantifier(&mut self, node: AstNode) -> Result<AstNode, ParseError> {
        self.skip_whitespace_and_comments();
        let quantified = match self.current() {
            Some(&'*') => {
                self.consume()?;
                let greedy = self.current() != Some(&'?');
                if !greedy {
                    self.consume()?;
                }
                AstNode::ZeroOrMore {
                    node: Box::new(node),
                    greedy,
                }
            }
            Some(&'+') => {
                self.consume()?;
                let greedy = self.current() != Some(&'?');
                if !greedy {
                    self.consume()?;
                }
                AstNode::OneOrMore {
                    node: Box::new(node),
                    greedy,
                }
            }
            Some(&'?') => {
                self.consume()?;
                let greedy = self.current() != Some(&'?');
                if !greedy {
                    self.consume()?;
                }
                AstNode::Optional {
                    node: Box::new(node),
                    greedy,
                }
            }
            Some(&'{') => self.parse_bounded_quantifier(node)?,
            _ => return Ok(node),
        };
        self.apply_quantifier(quantified)
    }

    // Parse {n}, {n,}, {n,m}, {,m}
    fn parse_bounded_quantifier(&mut self, node: AstNode) -> Result<AstNode, ParseError> {
        self.consume()?; // consume {

        // Parse min
        let min = if self.current() == Some(&',') {
            0
        } else {
            self.parse_number()?
        };

        match self.current() {
            Some(&',') => {
                self.consume()?;
                // Parse max (optional)
                let max = if self.current() == Some(&'}') {
                    None
                } else {
                    Some(self.parse_number()?)
                };

                if self.current() != Some(&'}') {
                    return Err(ParseError::InvalidQuantifier("expected '}'".to_string()));
                }
                self.consume()?;

                let greedy = self.current() != Some(&'?');
                if !greedy {
                    self.consume()?;
                }

                Ok(AstNode::Range {
                    node: Box::new(node),
                    min,
                    max,
                    greedy,
                })
            }
            Some(&'}') => {
                self.consume()?;
                if self.current() == Some(&'?') {
                    self.consume()?;
                }
                Ok(AstNode::Exact {
                    node: Box::new(node),
                    count: min,
                })
            }
            _ => Err(ParseError::InvalidQuantifier(
                "expected ',' or '}'".to_string(),
            )),
        }
    }

    // Helper: parse a decimal number
    fn parse_number(&mut self) -> Result<usize, ParseError> {
        let mut num = 0;
        let mut found = false;

        while let Some(&c @ '0'..='9') = self.current() {
            found = true;
            num = num * 10 + (c.to_digit(10).unwrap() as usize);
            self.consume()?;
        }

        if !found {
            return Err(ParseError::InvalidLineNumber("expected digits".to_string()));
        }

        Ok(num)
    }

    fn expect_close_paren(&mut self) -> Result<(), ParseError> {
        if self.current() != Some(&')') {
            return Err(ParseError::UnmatchedParen);
        }
        self.consume()?;
        Ok(())
    }

    // Helper: get current char without advancing
    fn current(&self) -> Option<&char> {
        self.input.get(self.pos)
    }

    // Helper: peek ahead n positions
    fn peek_ahead(&self, n: usize) -> Option<&char> {
        self.input.get(self.pos + n)
    }

    // Helper: peek next char
    fn peek(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    // Helper: consume current char and advance
    fn consume(&mut self) -> Result<char, ParseError> {
        match self.current() {
            Some(&ch) => {
                self.pos += 1;
                Ok(ch)
            }
            None => Err(ParseError::UnexpectedEof),
        }
    }
}

fn collect_group_names(nodes: &[AstNode], names: &mut HashMap<String, usize>) {
    for node in nodes {
        match node {
            AstNode::Group {
                nodes, name, index, ..
            } => {
                if let (Some(n), Some(i)) = (name, index) {
                    names.insert(n.clone(), *i);
                }
                collect_group_names(nodes, names);
            }
            AstNode::Alternation(alts) => {
                for alt in alts {
                    collect_group_names(alt, names);
                }
            }
            AstNode::ZeroOrMore { node, .. }
            | AstNode::OneOrMore { node, .. }
            | AstNode::Optional { node, .. }
            | AstNode::Exact { node, .. }
            | AstNode::Range { node, .. } => {
                collect_group_names(std::slice::from_ref(node), names);
            }
            AstNode::LookAhead { nodes, .. } | AstNode::LookBehind { nodes, .. } => {
                collect_group_names(nodes, names);
            }
            _ => {}
        }
    }
}

fn resolve_subroutines(
    nodes: &mut [AstNode],
    names: &HashMap<String, usize>,
    max_group: usize,
) -> Result<(), ParseError> {
    for node in nodes {
        match node {
            AstNode::Subroutine(target) => {
                if let SubroutineTarget::Name(n) = target {
                    let idx = names.get(n).copied().ok_or_else(|| {
                        ParseError::InvalidSubroutineReference(format!(
                            "no group named '{n}'"
                        ))
                    })?;
                    *target = SubroutineTarget::Group(idx);
                }
                if let SubroutineTarget::Group(n) = target
                    && (*n == 0 || *n > max_group)
                {
                    return Err(ParseError::InvalidSubroutineReference(format!(
                        "reference to non-existent group {n}"
                    )));
                }
            }
            AstNode::Group { nodes, .. } => resolve_subroutines(nodes, names, max_group)?,
            AstNode::Alternation(alts) => {
                for alt in alts {
                    resolve_subroutines(alt, names, max_group)?;
                }
            }
            AstNode::ZeroOrMore { node, .. }
            | AstNode::OneOrMore { node, .. }
            | AstNode::Optional { node, .. }
            | AstNode::Exact { node, .. }
            | AstNode::Range { node, .. } => {
                resolve_subroutines(std::slice::from_mut(node), names, max_group)?;
            }
            AstNode::LookAhead { nodes, .. } | AstNode::LookBehind { nodes, .. } => {
                resolve_subroutines(nodes, names, max_group)?;
            }
            _ => {}
        }
    }
    Ok(())
}
