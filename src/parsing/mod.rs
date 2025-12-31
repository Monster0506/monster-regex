// Helper: Parse Rift format "pattern/flags"
use crate::errors::ParseError;
use crate::flags::Flags;
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
