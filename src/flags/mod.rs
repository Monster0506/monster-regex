/// Configuration flags that modify the behavior of the regular expression engine.
#[derive(Default)]
pub struct Flags {
    /// Controls case sensitivity.
    /// - `None`: Smartcase (case-insensitive if pattern is all lowercase, sensitive otherwise).
    /// - `Some(true)`: Case-insensitive (`i` flag).
    /// - `Some(false)`: Case-sensitive (`c` flag).
    pub ignore_case: Option<bool>,
    /// If true, `^` and `$` match line boundaries (`\n`) instead of just the start/end of the text (`m` flag).
    pub multiline: bool,
    /// If true, `.` matches newlines (`s` flag).
    pub dotall: bool,
    /// If true, whitespace and comments in the pattern are ignored (`x` flag).
    pub verbose: bool,
    /// If true, enables Unicode support for character classes (`u` flag).
    pub unicode: bool,
    /// If true, indicates that the regex should match all occurrences (`g` flag).
    /// Note: This flag is often handled by the caller (e.g., `find_all` vs `find`), but is preserved here for parsing.
    pub global: bool,
}

