// Match result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
}

impl Match {
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn as_str<'a>(&self, text: &'a str) -> &'a str {
        &text[self.start..self.end]
    }
}

// Captures with named groups
#[derive(Debug, Clone)]
pub struct Captures {
    pub full_match: Match,
    pub groups: Vec<Option<Match>>, // groups[0] = \1, [1] = \2, etc
    pub named: std::collections::HashMap<String, Match>,
}

impl Captures {
    pub fn get(&self, index: usize) -> Option<&Match> {
        if index == 0 {
            Some(&self.full_match)
        } else {
            self.groups.get(index - 1).and_then(|g| g.as_ref())
        }
    }

    pub fn get_named(&self, name: &str) -> Option<&Match> {
        self.named.get(name)
    }

    pub fn as_str<'a>(&self, text: &'a str, index: usize) -> Option<&'a str> {
        self.get(index).map(|m| m.as_str(text))
    }

    pub fn as_str_named<'a>(&self, text: &'a str, name: &str) -> Option<&'a str> {
        self.get_named(name).map(|m| m.as_str(text))
    }
}
