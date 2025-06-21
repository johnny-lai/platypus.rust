use crate::protocol::*;
use anyhow::{Result, anyhow};

pub fn parse(line: &String) -> Result<Command> {
    let line = line.trim();
    if line.is_empty() {
        return Err(anyhow!("empty command"));
    }
    
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return Err(anyhow!("empty command"));
    }
    
    match parts[0] {
        // Retrieval commands
        "get" => {
            if parts.len() < 2 {
                return Err(anyhow!("get requires at least one key"));
            }
            let keys = parts[1..].iter().map(|s| s.to_string()).collect();
            Ok(Command::Get(keys))
        },
        "gets" => {
            if parts.len() < 2 {
                return Err(anyhow!("gets requires at least one key"));
            }
            let keys = parts[1..].iter().map(|s| s.to_string()).collect();
            Ok(Command::Gets(keys))
        },
        "gat" => {
            if parts.len() < 3 {
                return Err(anyhow!("gat requires exptime and at least one key"));
            }
            let exptime = parts[1].parse::<u32>()
                .map_err(|_| anyhow!("invalid exptime in gat command"))?;
            let keys = parts[2..].iter().map(|s| s.to_string()).collect();
            Ok(Command::Gat(exptime, keys))
        },
        "gats" => {
            if parts.len() < 3 {
                return Err(anyhow!("gats requires exptime and at least one key"));
            }
            let exptime = parts[1].parse::<u32>()
                .map_err(|_| anyhow!("invalid exptime in gats command"))?;
            let keys = parts[2..].iter().map(|s| s.to_string()).collect();
            Ok(Command::Gats(exptime, keys))
        },
        
        // Administrative commands
        "version" => Ok(Command::Version),
        "stats" => {
            let arg = if parts.len() > 1 {
                Some(parts[1..].join(" "))
            } else {
                None
            };
            Ok(Command::Stats(arg))
        },
        "touch" => {
            if parts.len() != 3 {
                return Err(anyhow!("touch requires key and exptime"));
            }
            let key = parts[1].to_string();
            let exptime = parts[2].parse::<u32>()
                .map_err(|_| anyhow!("invalid exptime in touch command"))?;
            Ok(Command::Touch(key, exptime))
        },
        "quit" => Ok(Command::Quit),
        
        _ => Err(anyhow!("unknown command: {}", parts[0])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_single_key() {
        let result = parse(&"get mykey".to_string()).unwrap();
        assert_eq!(result, Command::Get(vec!["mykey".to_string()]));
    }

    #[test]
    fn test_get_multiple_keys() {
        let result = parse(&"get key1 key2 key3".to_string()).unwrap();
        assert_eq!(result, Command::Get(vec!["key1".to_string(), "key2".to_string(), "key3".to_string()]));
    }

    #[test]
    fn test_gets_command() {
        let result = parse(&"gets mykey".to_string()).unwrap();
        assert_eq!(result, Command::Gets(vec!["mykey".to_string()]));
    }

    #[test]
    fn test_gat_command() {
        let result = parse(&"gat 3600 mykey".to_string()).unwrap();
        assert_eq!(result, Command::Gat(3600, vec!["mykey".to_string()]));
    }

    #[test]
    fn test_gats_command() {
        let result = parse(&"gats 3600 key1 key2".to_string()).unwrap();
        assert_eq!(result, Command::Gats(3600, vec!["key1".to_string(), "key2".to_string()]));
    }

    #[test]
    fn test_version_command() {
        let result = parse(&"version".to_string()).unwrap();
        assert_eq!(result, Command::Version);
    }

    #[test]
    fn test_stats_command() {
        let result = parse(&"stats".to_string()).unwrap();
        assert_eq!(result, Command::Stats(None));
    }

    #[test]
    fn test_stats_with_args() {
        let result = parse(&"stats slabs".to_string()).unwrap();
        assert_eq!(result, Command::Stats(Some("slabs".to_string())));
    }

    #[test]
    fn test_touch_command() {
        let result = parse(&"touch mykey 3600".to_string()).unwrap();
        assert_eq!(result, Command::Touch("mykey".to_string(), 3600));
    }

    #[test]
    fn test_quit_command() {
        let result = parse(&"quit".to_string()).unwrap();
        assert_eq!(result, Command::Quit);
    }

    #[test]
    fn test_invalid_command() {
        let result = parse(&"invalid".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_get_no_keys() {
        let result = parse(&"get".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_touch_invalid_exptime() {
        let result = parse(&"touch mykey invalid".to_string());
        assert!(result.is_err());
    }
}
