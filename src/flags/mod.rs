pub struct Flags {
    pub ignore_case: Option<bool>, // None = smartcase, Some(true) = ignore, Some(false) = case-sensitive
    pub multiline: bool,           // m
    pub dotall: bool,              // s
    pub verbose: bool,             // x
    pub unicode: bool,             // u
    pub global: bool,              // g
}

impl Default for Flags {
    fn default() -> Self {
        Flags {
            ignore_case: None,
            multiline: false,
            dotall: false,
            verbose: false,
            unicode: false,
            global: false,
        }
    }
}
