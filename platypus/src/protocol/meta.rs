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
        "mg" => {
            if parts.len() < 2 {
                return Err(anyhow!("mg requires key"));
            }
            let key = parts[1].to_string();
            let flags = if parts.len() > 2 {
                parse_meta_flags(&parts[2..])?
            } else {
                Vec::new()
            };
            Ok(Command::MetaGet(key, flags))
        },
        "mn" => {
            if parts.len() != 1 {
                return Err(anyhow!("mn takes no arguments"));
            }
            Ok(Command::MetaNoOp)
        },
        _ => Err(anyhow!("unknown meta command: {}", parts[0])),
    }
}

fn parse_meta_flags(flag_parts: &[&str]) -> Result<Vec<MetaFlag>> {
    let mut flags = Vec::new();
    
    for part in flag_parts {
        // Handle flags with tokens
        if part.starts_with('O') {
            let token = part[1..].to_string();
            if token.is_empty() {
                return Err(anyhow!("O flag requires token"));
            }
            flags.push(MetaFlag::Opaque(token));
        } else if part.starts_with('N') {
            let token = &part[1..];
            let ttl = token.parse::<u32>()
                .map_err(|_| anyhow!("N flag requires numeric TTL"))?;
            flags.push(MetaFlag::VivifyOnMiss(ttl));
        } else if part.starts_with('R') {
            let token = &part[1..];
            let ttl = token.parse::<u32>()
                .map_err(|_| anyhow!("R flag requires numeric TTL"))?;
            flags.push(MetaFlag::RecacheWin(ttl));
        } else if part.starts_with('T') {
            let token = &part[1..];
            let ttl = token.parse::<u32>()
                .map_err(|_| anyhow!("T flag requires numeric TTL"))?;
            flags.push(MetaFlag::UpdateTtl(ttl));
        } else if part.starts_with('E') {
            let token = &part[1..];
            let cas = token.parse::<u64>()
                .map_err(|_| anyhow!("E flag requires numeric CAS"))?;
            flags.push(MetaFlag::SetCas(cas));
        } else {
            // Handle single character flags
            for ch in part.chars() {
                match ch {
                    'b' => flags.push(MetaFlag::BaseEncoded),
                    'c' => flags.push(MetaFlag::ReturnCas),
                    'f' => flags.push(MetaFlag::ReturnFlags),
                    'h' => flags.push(MetaFlag::ReturnHit),
                    'k' => flags.push(MetaFlag::ReturnKey),
                    'l' => flags.push(MetaFlag::ReturnLastAccess),
                    'q' => flags.push(MetaFlag::NoReply),
                    's' => flags.push(MetaFlag::ReturnSize),
                    't' => flags.push(MetaFlag::ReturnTtl),
                    'u' => flags.push(MetaFlag::NoLruBump),
                    'v' => flags.push(MetaFlag::ReturnValue),
                    _ => return Err(anyhow!("unknown meta flag: {}", ch)),
                }
            }
        }
    }
    
    Ok(flags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mg_basic() {
        let result = parse(&"mg mykey".to_string()).unwrap();
        assert_eq!(result, Command::MetaGet("mykey".to_string(), vec![]));
    }

    #[test]
    fn test_mg_with_simple_flags() {
        let result = parse(&"mg mykey v".to_string()).unwrap();
        assert_eq!(result, Command::MetaGet("mykey".to_string(), vec![MetaFlag::ReturnValue]));
    }

    #[test]
    fn test_mg_with_multiple_flags() {
        let result = parse(&"mg mykey vck".to_string()).unwrap();
        assert_eq!(result, Command::MetaGet("mykey".to_string(), vec![
            MetaFlag::ReturnValue,
            MetaFlag::ReturnCas,
            MetaFlag::ReturnKey
        ]));
    }

    #[test]
    fn test_mg_with_opaque_flag() {
        let result = parse(&"mg mykey v Otest123".to_string()).unwrap();
        assert_eq!(result, Command::MetaGet("mykey".to_string(), vec![
            MetaFlag::ReturnValue,
            MetaFlag::Opaque("test123".to_string())
        ]));
    }

    #[test]
    fn test_mg_with_ttl_flags() {
        let result = parse(&"mg mykey N3600 R1800".to_string()).unwrap();
        assert_eq!(result, Command::MetaGet("mykey".to_string(), vec![
            MetaFlag::VivifyOnMiss(3600),
            MetaFlag::RecacheWin(1800)
        ]));
    }

    #[test]
    fn test_mn_command() {
        let result = parse(&"mn".to_string()).unwrap();
        assert_eq!(result, Command::MetaNoOp);
    }

    #[test]
    fn test_mg_no_key() {
        let result = parse(&"mg".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_mn_with_args() {
        let result = parse(&"mn extra".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_meta_command() {
        let result = parse(&"mx mykey".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_flag() {
        let result = parse(&"mg mykey x".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_ttl_flag() {
        let result = parse(&"mg mykey Ninvalid".to_string());
        assert!(result.is_err());
    }
}
