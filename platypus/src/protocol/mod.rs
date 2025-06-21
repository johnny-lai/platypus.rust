pub mod meta;
pub mod text;

#[derive(Debug, PartialEq)]
pub enum Command {
    // Retrieval commands
    Get(Vec<String>),
    Gets(Vec<String>), 
    Gat(u32, Vec<String>), // (exptime, keys)
    Gats(u32, Vec<String>), // (exptime, keys)
    
    // Meta commands
    MetaGet(String, Vec<MetaFlag>), // (key, flags)
    MetaNoOp,
    
    // Administrative commands
    Version,
    Stats(Option<String>), // optional argument
    Touch(String, u32), // (key, exptime)
    Quit,
}

#[derive(Debug, PartialEq, Clone)]
pub enum MetaFlag {
    // Flags without tokens
    BaseEncoded, // b
    ReturnCas, // c
    ReturnFlags, // f
    ReturnHit, // h
    ReturnKey, // k
    ReturnLastAccess, // l
    NoReply, // q
    ReturnSize, // s
    ReturnTtl, // t
    NoLruBump, // u
    ReturnValue, // v
    
    // Flags with tokens
    Opaque(String), // O(token)
    VivifyOnMiss(u32), // N(token) - TTL
    RecacheWin(u32), // R(token) - TTL threshold
    UpdateTtl(u32), // T(token) - new TTL
    SetCas(u64), // E(token) - CAS value
}

#[derive(Debug, PartialEq)]
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
    MetaHit(Vec<MetaFlag>), // HD response
    MetaEnd, // EN response
    MetaNoOp, // MN response
}

impl Response {
    pub fn format(&self) -> String {
        match self {
            Response::Value(item) => {
                format!("VALUE {} {} {}\r\n{}\r\n", 
                    item.key, 
                    item.flags, 
                    item.data.len(),
                    String::from_utf8_lossy(&item.data)
                )
            },
            Response::Values(items) => {
                let mut result = String::new();
                for item in items {
                    result.push_str(&format!("VALUE {} {} {}", 
                        item.key, 
                        item.flags, 
                        item.data.len()
                    ));
                    if let Some(cas) = item.cas {
                        result.push_str(&format!(" {}", cas));
                    }
                    result.push_str(&format!("\r\n{}\r\n", 
                        String::from_utf8_lossy(&item.data)
                    ));
                }
                result.push_str("END\r\n");
                result
            },
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
            },
            // Meta responses
            Response::MetaValue(item, flags) => {
                let mut result = format!("VA {}", item.data.len());
                for flag in flags {
                    result.push(' ');
                    result.push_str(&flag.format_response());
                }
                result.push_str(&format!("\r\n{}\r\n", 
                    String::from_utf8_lossy(&item.data)
                ));
                result
            },
            Response::MetaHit(flags) => {
                let mut result = "HD".to_string();
                for flag in flags {
                    result.push(' ');
                    result.push_str(&flag.format_response());
                }
                result.push_str("\r\n");
                result
            },
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

pub fn parse(line: &str) -> anyhow::Result<Command> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("empty command"));
    }
    
    // Try meta commands first (mg, mn)
    if trimmed.starts_with("mg ") || trimmed == "mn" {
        return meta::parse(&trimmed.to_string());
    }
    
    // Fall back to text protocol
    text::parse(&trimmed.to_string())
}
