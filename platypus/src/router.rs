use crate::Request;
use regex::Regex;

macro_rules! panic_on_err {
    ($expr:expr) => {
        match $expr {
            Ok(x) => x,
            Err(err) => panic!("{err}"),
        }
    };
}

pub enum Error {}

pub struct Rule {
    patten: Regex,
    source: String,
}

impl Rule {
    pub fn new(pattern: &str, source: impl Into<String>) -> Result<Self, regex::Error> {
        let re = Regex::new(pattern)?;
        Ok(Self {
            patten: re,
            source: source.into(),
        })
    }

    pub fn match_key(&self, key: &str) -> Option<Request> {
        Request::match_regex(&self.patten, key)
    }

    pub fn source(&self) -> &String {
        &self.source
    }
}

pub struct Router {
    rules: std::collections::LinkedList<Rule>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            rules: std::collections::LinkedList::new(),
        }
    }

    pub fn route(mut self, pattern: &str, source: impl Into<String>) -> Self {
        let rule = panic_on_err!(Rule::new(pattern, source));
        self.rules.push_back(rule);
        self
    }

    pub fn rule(&self, key: &str) -> Option<(Request, &Rule)> {
        for rule in self.rules.iter() {
            if let Some(request) = rule.match_key(key) {
                return Some((request, &rule));
            }
        }
        None
    }
}
