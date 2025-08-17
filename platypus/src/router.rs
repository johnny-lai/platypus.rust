use crate::Source;
use crate::request::Request;
use regex::Regex;
use std::sync::Arc;

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
    re: Regex,
    source: Arc<Box<dyn Source>>,
}

impl Rule {
    pub fn new(pattern: &str, source: Arc<Box<dyn Source>>) -> Result<Self, regex::Error> {
        let re = Regex::new(pattern)?;
        Ok(Self { re, source })
    }

    pub fn match_key(&self, key: &str) -> Option<Request> {
        Request::match_regex(&self.re, key)
    }

    pub fn source(&self) -> Arc<Box<dyn Source>> {
        self.source.clone()
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

    pub fn route(mut self, pattern: &str, source: Box<dyn Source + Send + 'static>) -> Self {
        let rule = panic_on_err!(Rule::new(pattern, Arc::new(source)));
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
