use anyhow::{Result, anyhow};
use thiserror::Error;
use tokio::io::AsyncBufReadExt;

pub mod binary;
pub mod meta;
pub mod text;

#[derive(Debug, PartialEq, Clone)]
pub enum ProtocolType {
    Text,
    Binary { opaque: u32 },
    Meta,
}

#[derive(Debug, PartialEq)]
pub struct CommandContext {
    pub command: Command,
    pub protocol: ProtocolType,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    // Retrieval commands
    Get(Vec<String>),
    Gets(Vec<String>),
    Gat(u32, Vec<String>),  // (exptime, keys)
    Gats(u32, Vec<String>), // (exptime, keys)

    // Meta commands
    MetaGet(String, Vec<MetaFlag>), // (key, flags)
    MetaNoOp,

    // Administrative commands
    Version,
    Stats(Option<String>), // optional argument
    Touch(String, u32),    // (key, exptime)
    Quit,
}

#[derive(Debug, PartialEq, Clone)]
pub enum MetaFlag {
    // Flags without tokens
    BaseEncoded,      // b
    ReturnCas,        // c
    ReturnFlags,      // f
    ReturnHit,        // h
    ReturnKey,        // k
    ReturnLastAccess, // l
    NoReply,          // q
    ReturnSize,       // s
    ReturnTtl,        // t
    NoLruBump,        // u
    ReturnValue,      // v

    // Flags with tokens
    Opaque(String),    // O(token)
    VivifyOnMiss(u32), // N(token) - TTL
    RecacheWin(u32),   // R(token) - TTL threshold
    UpdateTtl(u32),    // T(token) - new TTL
    SetCas(u64),       // E(token) - CAS value
}

#[derive(Debug, PartialEq, Clone)]
pub struct Item {
    pub key: String,
    pub flags: u32,
    pub exptime: u32,
    pub data: Vec<u8>,
    pub cas: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub enum Response {
    Value(Item),
    Values(Vec<Item>),
    End,
    Stored,
    NotStored,
    Exists,
    NotFound,
    Deleted,
    Touched,
    Error(String),
    ClientError(String),
    ServerError(String),
    Version(String),
    Stats(Vec<(String, String)>),
    // Meta responses
    MetaValue(Item, Vec<MetaFlag>), // VA response
    MetaHit(Vec<MetaFlag>),         // HD response
    MetaEnd,                        // EN response
    MetaNoOp,                       // MN response
}

impl Response {
    pub fn serialize(&self, protocol: &ProtocolType) -> Vec<u8> {
        match protocol {
            ProtocolType::Text | ProtocolType::Meta => self.format().into_bytes(),
            ProtocolType::Binary { opaque } => self.serialize_binary(*opaque),
        }
    }

    fn serialize_binary(&self, opaque: u32) -> Vec<u8> {
        binary::serialize_binary_response(self, opaque).unwrap_or_else(|_| {
            // Fallback to a simple error response if serialization fails
            b"ERROR\r\n".to_vec()
        })
    }

    pub fn format(&self) -> String {
        match self {
            Response::Value(item) => {
                format!(
                    "VALUE {} {} {}\r\n{}\r\n",
                    item.key,
                    item.flags,
                    item.data.len(),
                    String::from_utf8_lossy(&item.data)
                )
            }
            Response::Values(items) => {
                let mut result = String::new();
                for item in items {
                    result.push_str(&format!(
                        "VALUE {} {} {}",
                        item.key,
                        item.flags,
                        item.data.len()
                    ));
                    if let Some(cas) = item.cas {
                        result.push_str(&format!(" {}", cas));
                    }
                    result.push_str(&format!("\r\n{}\r\n", String::from_utf8_lossy(&item.data)));
                }
                result.push_str("END\r\n");
                result
            }
            Response::End => "END\r\n".to_string(),
            Response::Stored => "STORED\r\n".to_string(),
            Response::NotStored => "NOT_STORED\r\n".to_string(),
            Response::Exists => "EXISTS\r\n".to_string(),
            Response::NotFound => "NOT_FOUND\r\n".to_string(),
            Response::Deleted => "DELETED\r\n".to_string(),
            Response::Touched => "TOUCHED\r\n".to_string(),
            Response::Error(msg) => format!("ERROR {}\r\n", msg),
            Response::ClientError(msg) => format!("CLIENT_ERROR {}\r\n", msg),
            Response::ServerError(msg) => format!("SERVER_ERROR {}\r\n", msg),
            Response::Version(version) => format!("VERSION {}\r\n", version),
            Response::Stats(stats) => {
                let mut result = String::new();
                for (key, value) in stats {
                    result.push_str(&format!("STAT {} {}\r\n", key, value));
                }
                result.push_str("END\r\n");
                result
            }
            // Meta responses
            Response::MetaValue(item, flags) => {
                let mut result = format!("VA {}", item.data.len());
                for flag in flags {
                    result.push(' ');
                    result.push_str(&flag.format_response());
                }
                result.push_str(&format!("\r\n{}\r\n", String::from_utf8_lossy(&item.data)));
                result
            }
            Response::MetaHit(flags) => {
                let mut result = "HD".to_string();
                for flag in flags {
                    result.push(' ');
                    result.push_str(&flag.format_response());
                }
                result.push_str("\r\n");
                result
            }
            Response::MetaEnd => "EN\r\n".to_string(),
            Response::MetaNoOp => "MN\r\n".to_string(),
        }
    }
}

impl MetaFlag {
    pub fn format_response(&self) -> String {
        match self {
            MetaFlag::BaseEncoded => "b".to_string(),
            MetaFlag::ReturnCas => "c".to_string(),
            MetaFlag::ReturnFlags => "f".to_string(),
            MetaFlag::ReturnHit => "h".to_string(),
            MetaFlag::ReturnKey => "k".to_string(),
            MetaFlag::ReturnLastAccess => "l".to_string(),
            MetaFlag::NoReply => "q".to_string(),
            MetaFlag::ReturnSize => "s".to_string(),
            MetaFlag::ReturnTtl => "t".to_string(),
            MetaFlag::NoLruBump => "u".to_string(),
            MetaFlag::ReturnValue => "v".to_string(),
            MetaFlag::Opaque(token) => format!("O{}", token),
            MetaFlag::VivifyOnMiss(ttl) => format!("N{}", ttl),
            MetaFlag::RecacheWin(ttl) => format!("R{}", ttl),
            MetaFlag::UpdateTtl(ttl) => format!("T{}", ttl),
            MetaFlag::SetCas(cas) => format!("E{}", cas),
        }
    }
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("no command")]
    NoCommand,

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub async fn recv_command<R>(reader: &mut R) -> Result<CommandContext>
where
    R: AsyncBufReadExt + Unpin,
{
    let data = reader.fill_buf().await?;
    if data.len() < 1 {
        return Err(ParseError::NoCommand.into());
    } else if data[0] == 0x80 || data[0] == 0x81 {
        // Binary protocol - magic byte 0x80 (request) or 0x81 (response)
        // Header is 24 bytes
        match binary::parse_binary(data) {
            Ok((command, opaque, consumed)) => {
                reader.consume(consumed);
                Ok(CommandContext {
                    command,
                    protocol: ProtocolType::Binary { opaque },
                })
            }
            Err(err) => Err(err),
        }
    } else {
        // Text or Meta protocol
        let mut line = String::new();
        _ = reader.read_line(&mut line).await?;
        parse_text(&line)
    }
}

/// Parse text-based protocol (text or meta)
pub fn parse_text(line: &str) -> anyhow::Result<CommandContext> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(ParseError::NoCommand.into());
    }

    // Try meta commands first (mg, mn)
    if trimmed.starts_with("mg ") || trimmed == "mn" {
        let command = meta::parse(&trimmed.to_string())?;
        Ok(CommandContext {
            protocol: ProtocolType::Meta,
            command,
        })
    } else {
        // Fall back to text protocol
        let command = text::parse(&trimmed.to_string())?;
        Ok(CommandContext {
            protocol: ProtocolType::Text,
            command,
        })
    }
}

/// Parse any protocol type (binary, text, or meta) from raw bytes
pub fn parse_any(data: &[u8]) -> anyhow::Result<Command> {
    if data.is_empty() {
        return Err(anyhow!("empty data"));
    }

    // Check if it's binary protocol by looking at the magic byte
    if data[0] == 0x80 || data[0] == 0x81 {
        // Binary protocol - magic byte 0x80 (request) or 0x81 (response)
        return match binary::parse_binary(data) {
            Ok((command, _, _)) => Ok(command),
            Err(err) => Err(err),
        };
    }

    // Try to parse as text/meta protocol
    let text = std::str::from_utf8(data).map_err(|_| anyhow!("Invalid UTF-8 in text protocol"))?;

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ParseError::NoCommand.into());
    }

    // Try meta commands first (mg, mn)
    if trimmed.starts_with("mg ") || trimmed == "mn" {
        return meta::parse(&trimmed.to_string());
    }

    // Fall back to text protocol
    text::parse(&trimmed.to_string())
}

pub fn serialize_binary_response(response: &Response, opaque: u32) -> anyhow::Result<Vec<u8>> {
    binary::serialize_binary_response(response, opaque)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_any_binary_protocol() {
        // Create a binary GET request for key "test"
        let mut packet = Vec::new();
        let header = binary::BinaryHeader::new_request(0x00, 4, 0, 4); // GET opcode, key_length=4
        header.write_to(&mut packet).unwrap();
        packet.extend_from_slice(b"test");

        let cmd = parse_any(&packet).unwrap();
        match cmd {
            Command::Get(keys) => {
                assert_eq!(keys.len(), 1);
                assert_eq!(keys[0], "test");
            }
            _ => panic!("Expected Get command"),
        }
    }

    #[test]
    fn test_parse_any_text_protocol() {
        let data = b"get test\r\n";
        let cmd = parse_any(data).unwrap();
        match cmd {
            Command::Get(keys) => {
                assert_eq!(keys.len(), 1);
                assert_eq!(keys[0], "test");
            }
            _ => panic!("Expected Get command"),
        }
    }

    #[test]
    fn test_parse_any_meta_protocol() {
        let data = b"mg test v\r\n";
        let cmd = parse_any(data).unwrap();
        match cmd {
            Command::MetaGet(key, flags) => {
                assert_eq!(key, "test");
                assert_eq!(flags.len(), 1);
                assert_eq!(flags[0], MetaFlag::ReturnValue);
            }
            _ => panic!("Expected MetaGet command"),
        }
    }

    #[test]
    fn test_parse_any_version_text() {
        let data = b"version\r\n";
        let cmd = parse_any(data).unwrap();
        assert!(matches!(cmd, Command::Version));
    }

    #[test]
    fn test_parse_any_version_binary() {
        let mut packet = Vec::new();
        let header = binary::BinaryHeader::new_request(0x0b, 0, 0, 0); // VERSION opcode
        header.write_to(&mut packet).unwrap();

        let cmd = parse_any(&packet).unwrap();
        assert!(matches!(cmd, Command::Version));
    }

    #[test]
    fn test_parse_any_invalid_utf8() {
        let data = &[0xff, 0xfe, 0xfd]; // Invalid UTF-8, not binary magic
        let result = parse_any(data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid UTF-8"));
    }

    #[test]
    fn test_parse_any_empty_data() {
        let data = &[];
        let result = parse_any(data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty data"));
    }
}
