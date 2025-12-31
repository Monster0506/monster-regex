use crate::errors::ParseError;
use crate::flags::Flags;

/// Parses a string in the Rift format: `pattern/flags`.
///
/// This format expects the pattern to be terminated by a forward slash `/`,
/// followed by any number of single-character flags.
///
/// # Flags
///
/// * `i`: Ignore case (case-insensitive).
/// * `c`: Case-sensitive.
/// * `m`: Multiline mode (`^` and `$` match line boundaries).
/// * `s`: Dotall mode (`.` matches newlines).
/// * `x`: Verbose mode (whitespace and comments ignored).
/// * `u`: Unicode support.
/// * `g`: Global match.
///
/// # Smartcase
///
/// If neither `i` nor `c` is specified, the case sensitivity is inferred:
/// * If the pattern contains any uppercase characters, it defaults to case-sensitive.
/// * Otherwise, it defaults to case-insensitive.
///
/// # Errors
///
/// Returns `ParseError::NoDelimiter` if the input string does not contain a `/`.
/// Returns `ParseError::InvalidFlags` if an unknown flag character is encountered.
pub fn parse_rift_format(input: &str) -> Result<(String, Flags), ParseError> {
    let last_slash = input.rfind('/').ok_or(ParseError::NoDelimiter)?;

    let pattern = &input[..last_slash];
    let flag_str = &input[last_slash + 1..];

    let mut flags = Flags::default();

    for ch in flag_str.chars() {
        match ch {
            'i' => flags.ignore_case = Some(true),
            'c' => flags.ignore_case = Some(false),
            'm' => flags.multiline = true,
            's' => flags.dotall = true,
            'x' => flags.verbose = true,
            'u' => flags.unicode = true,
            'g' => flags.global = true,
            _ => return Err(ParseError::InvalidFlags(ch)),
        }
    }

    // Smartcase: if no explicit case flag, infer from pattern
    if flags.ignore_case.is_none() {
        let has_uppercase = pattern.chars().any(|c| c.is_uppercase());
        flags.ignore_case = Some(!has_uppercase);
    }

    Ok((pattern.to_string(), flags))
}
