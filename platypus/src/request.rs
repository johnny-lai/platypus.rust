use regex::Regex;
use std::collections::HashMap;

pub struct Request {
    key: String,
    captures: HashMap<String, String>,
}

impl Request {
    pub fn new(key: &str) -> Self {
        Self {
            key: key.into(),
            captures: HashMap::new(),
        }
    }
    pub fn match_regex(re: &Regex, key: &str) -> Option<Self> {
        let Some(caps) = re.captures(key) else {
            return None;
        };
        let mut captures = HashMap::new();
        for name in re.capture_names().flatten() {
            if let Some(matched) = caps.name(name) {
                captures.insert(name.into(), matched.as_str().into());
            }
        }
        captures.insert("$key".to_string(), key.to_string());
        Some(Self {
            key: key.into(),
            captures,
        })
    }

    // Returns the entire key
    pub fn key(&self) -> &str {
        self.key.as_str()
    }

    // Returns a named captured value
    pub fn get(&self, k: &str) -> Option<&str> {
        self.captures.get(k).map(|s| s.as_str())
    }

    pub fn captures(&self) -> &HashMap<String, String> {
        &self.captures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_regex_no_match() {
        let re = Regex::new(r"^(?<instance>[^/]+)/both$").unwrap();
        let request = Request::match_regex(&re, "abc/i_dont_match");
        assert!(request.is_none());
    }

    #[test]
    fn test_match_regex_match() {
        let re = Regex::new(r"^(?<instance>[^/]+)/both$").unwrap();
        let request = Request::match_regex(&re, "abc/both");
        assert!(request.is_some());
        let request = request.unwrap();
        assert_eq!(request.key(), "abc/both");
        assert_eq!(request.get("instance"), Some("abc"));
    }
}
