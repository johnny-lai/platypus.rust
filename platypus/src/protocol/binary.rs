use anyhow::{Result, anyhow};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Cursor, Read, Write};

use super::{Command, Response};

// Binary protocol pub constants
pub const MAGIC_REQUEST: u8 = 0x80;
pub const MAGIC_RESPONSE: u8 = 0x81;

// Opcodes for commands that exist in our protocol
pub const OPCODE_GET: u8 = 0x00;
pub const OPCODE_SET: u8 = 0x01;
#[allow(dead_code)]
pub const OPCODE_ADD: u8 = 0x02;
#[allow(dead_code)]
pub const OPCODE_REPLACE: u8 = 0x03;
pub const OPCODE_DELETE: u8 = 0x04;
#[allow(dead_code)]
pub const OPCODE_INCREMENT: u8 = 0x05;
#[allow(dead_code)]
pub const OPCODE_DECREMENT: u8 = 0x06;
pub const OPCODE_QUIT: u8 = 0x07;
#[allow(dead_code)]
pub const OPCODE_FLUSH: u8 = 0x08;
#[allow(dead_code)]
pub const OPCODE_GETQ: u8 = 0x09;
pub const OPCODE_NOOP: u8 = 0x0a;
pub const OPCODE_VERSION: u8 = 0x0b;
pub const OPCODE_GETK: u8 = 0x0c;
#[allow(dead_code)]
pub const OPCODE_GETKQ: u8 = 0x0d;
#[allow(dead_code)]
pub const OPCODE_APPEND: u8 = 0x0e;
#[allow(dead_code)]
pub const OPCODE_PREPEND: u8 = 0x0f;
pub const OPCODE_STAT: u8 = 0x10;

// Response status codes
pub const STATUS_SUCCESS: u16 = 0x0000;
pub const STATUS_KEY_NOT_FOUND: u16 = 0x0001;
pub const STATUS_KEY_EXISTS: u16 = 0x0002;
#[allow(dead_code)]
pub const STATUS_VALUE_TOO_LARGE: u16 = 0x0003;
pub const STATUS_INVALID_ARGUMENTS: u16 = 0x0004;
pub const STATUS_ITEM_NOT_STORED: u16 = 0x0005;
#[allow(dead_code)]
pub const STATUS_INCR_DECR_NON_NUMERIC: u16 = 0x0006;
pub const STATUS_UNKNOWN_COMMAND: u16 = 0x0081;
pub const STATUS_OUT_OF_MEMORY: u16 = 0x0082;

// Binary packet header structure
#[derive(Debug, Clone)]
pub struct BinaryHeader {
    pub magic: u8,
    pub opcode: u8,
    pub key_length: u16,
    pub extras_length: u8,
    pub data_type: u8,
    pub status_or_reserved: u16,
    pub total_body_length: u32,
    pub opaque: u32,
    pub cas: u64,
}

impl BinaryHeader {
    pub fn new_request(
        opcode: u8,
        key_length: u16,
        extras_length: u8,
        total_body_length: u32,
    ) -> Self {
        Self {
            magic: MAGIC_REQUEST,
            opcode,
            key_length,
            extras_length,
            data_type: 0,
            status_or_reserved: 0,
            total_body_length,
            opaque: 0,
            cas: 0,
        }
    }

    pub fn new_response(
        opcode: u8,
        key_length: u16,
        extras_length: u8,
        status: u16,
        total_body_length: u32,
        cas: u64,
    ) -> Self {
        Self {
            magic: MAGIC_RESPONSE,
            opcode,
            key_length,
            extras_length,
            data_type: 0,
            status_or_reserved: status,
            total_body_length,
            opaque: 0,
            cas,
        }
    }

    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let magic = reader.read_u8()?;
        let opcode = reader.read_u8()?;
        let key_length = reader.read_u16::<BigEndian>()?;
        let extras_length = reader.read_u8()?;
        let data_type = reader.read_u8()?;
        let status_or_reserved = reader.read_u16::<BigEndian>()?;
        let total_body_length = reader.read_u32::<BigEndian>()?;
        let opaque = reader.read_u32::<BigEndian>()?;
        let cas = reader.read_u64::<BigEndian>()?;

        Ok(Self {
            magic,
            opcode,
            key_length,
            extras_length,
            data_type,
            status_or_reserved,
            total_body_length,
            opaque,
            cas,
        })
    }

    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_u8(self.magic)?;
        writer.write_u8(self.opcode)?;
        writer.write_u16::<BigEndian>(self.key_length)?;
        writer.write_u8(self.extras_length)?;
        writer.write_u8(self.data_type)?;
        writer.write_u16::<BigEndian>(self.status_or_reserved)?;
        writer.write_u32::<BigEndian>(self.total_body_length)?;
        writer.write_u32::<BigEndian>(self.opaque)?;
        writer.write_u64::<BigEndian>(self.cas)?;
        Ok(())
    }
}

/// Returns the command, opaque and number of bytes used in data.
/// opaque will be copied back into the response.
pub fn parse_binary(data: &[u8]) -> Result<(Command, u32, usize)> {
    if data.len() < 24 {
        return Err(anyhow!("Binary packet too small"));
    }

    let mut cursor = Cursor::new(data);
    let header = BinaryHeader::read_from(&mut cursor)?;

    if header.magic != MAGIC_REQUEST {
        return Err(anyhow!("Invalid magic byte for request"));
    }

    // Read opaque
    let opaque = header.opaque;

    // Read body components
    let mut extras = vec![0u8; header.extras_length as usize];
    let mut key = vec![0u8; header.key_length as usize];
    let value_length = header.total_body_length as usize
        - header.extras_length as usize
        - header.key_length as usize;
    let mut value = vec![0u8; value_length];

    cursor.read_exact(&mut extras)?;
    cursor.read_exact(&mut key)?;
    cursor.read_exact(&mut value)?;

    let key_str = String::from_utf8(key).map_err(|_| anyhow!("Invalid key encoding"))?;

    match header.opcode {
        OPCODE_GET | OPCODE_GETK => {
            if header.extras_length != 0 {
                return Err(anyhow!("Get command must not have extras"));
            }
            if header.key_length == 0 {
                return Err(anyhow!("Get command must have key"));
            }
            if value_length != 0 {
                return Err(anyhow!("Get command must not have value"));
            }
            Ok((
                Command::Get(vec![key_str]),
                opaque,
                cursor.position() as usize,
            ))
        }
        OPCODE_VERSION => {
            if header.extras_length != 0 || header.key_length != 0 || value_length != 0 {
                return Err(anyhow!(
                    "Version command must not have extras, key, or value"
                ));
            }
            Ok((Command::Version, opaque, cursor.position() as usize))
        }
        OPCODE_QUIT => {
            if header.extras_length != 0 || header.key_length != 0 || value_length != 0 {
                return Err(anyhow!("Quit command must not have extras, key, or value"));
            }
            Ok((Command::Quit, opaque, cursor.position() as usize))
        }
        OPCODE_STAT => {
            if header.extras_length != 0 {
                return Err(anyhow!("Stat command must not have extras"));
            }
            if value_length != 0 {
                return Err(anyhow!("Stat command must not have value"));
            }
            let stats_arg = if header.key_length > 0 {
                Some(key_str)
            } else {
                None
            };
            Ok((
                Command::Stats(stats_arg),
                opaque,
                cursor.position() as usize,
            ))
        }
        OPCODE_DELETE => {
            if header.extras_length != 0 {
                return Err(anyhow!("Delete command must not have extras"));
            }
            if header.key_length == 0 {
                return Err(anyhow!("Delete command must have key"));
            }
            if value_length != 0 {
                return Err(anyhow!("Delete command must not have value"));
            }
            // Note: Binary protocol doesn't support touch directly, using placeholder exptime
            Ok((
                Command::Touch(key_str, 0),
                opaque,
                cursor.position() as usize,
            ))
        }
        _ => Err(anyhow!(
            "Unsupported binary command opcode: {}",
            header.opcode
        )),
    }
}

pub fn serialize_binary_response(response: &Response, opaque: u32) -> Result<Vec<u8>> {
    let mut result = Vec::new();

    match response {
        Response::Value(item) => {
            // Get response with value
            let extras = item.flags.to_be_bytes();
            let _key_bytes = item.key.as_bytes();
            let value_bytes = &item.data;

            let header = BinaryHeader {
                magic: MAGIC_RESPONSE,
                opcode: OPCODE_GET,
                key_length: 0, // Standard get doesn't return key
                extras_length: 4,
                data_type: 0,
                status_or_reserved: STATUS_SUCCESS,
                total_body_length: (4 + value_bytes.len()) as u32,
                opaque,
                cas: item.cas.unwrap_or(0),
            };

            header.write_to(&mut result)?;
            result.extend_from_slice(&extras);
            result.extend_from_slice(value_bytes);
        }
        Response::Values(items) => {
            // For multiple values, we need to send multiple response packets
            // This is a simplified implementation - in practice, binary protocol
            // handles this differently with quiet commands
            for item in items {
                let item_response = Response::Value(item.clone());
                let item_data = serialize_binary_response(&item_response, opaque)?;
                result.extend_from_slice(&item_data);
            }
        }
        Response::End => {
            // Empty response packet to signal end
            let header = BinaryHeader::new_response(OPCODE_GET, 0, 0, STATUS_SUCCESS, 0, 0);
            header.write_to(&mut result)?;
        }
        Response::NotFound => {
            let header = BinaryHeader::new_response(OPCODE_GET, 0, 0, STATUS_KEY_NOT_FOUND, 0, 0);
            header.write_to(&mut result)?;
        }
        Response::Version(version) => {
            let version_bytes = version.as_bytes();
            let header = BinaryHeader::new_response(
                OPCODE_VERSION,
                0,
                0,
                STATUS_SUCCESS,
                version_bytes.len() as u32,
                0,
            );
            header.write_to(&mut result)?;
            result.extend_from_slice(version_bytes);
        }
        Response::Stats(stats) => {
            // Send each stat as a separate packet
            for (key, value) in stats {
                let key_bytes = key.as_bytes();
                let value_bytes = value.as_bytes();
                let header = BinaryHeader::new_response(
                    OPCODE_STAT,
                    key_bytes.len() as u16,
                    0,
                    STATUS_SUCCESS,
                    (key_bytes.len() + value_bytes.len()) as u32,
                    0,
                );
                header.write_to(&mut result)?;
                result.extend_from_slice(key_bytes);
                result.extend_from_slice(value_bytes);
            }
            // Send terminating packet with no key/value
            let header = BinaryHeader::new_response(OPCODE_STAT, 0, 0, STATUS_SUCCESS, 0, 0);
            header.write_to(&mut result)?;
        }
        Response::Error(_) => {
            let header =
                BinaryHeader::new_response(OPCODE_NOOP, 0, 0, STATUS_UNKNOWN_COMMAND, 0, 0);
            header.write_to(&mut result)?;
        }
        Response::ClientError(_) => {
            let header =
                BinaryHeader::new_response(OPCODE_NOOP, 0, 0, STATUS_INVALID_ARGUMENTS, 0, 0);
            header.write_to(&mut result)?;
        }
        Response::ServerError(_) => {
            let header = BinaryHeader::new_response(OPCODE_NOOP, 0, 0, STATUS_OUT_OF_MEMORY, 0, 0);
            header.write_to(&mut result)?;
        }
        // Simple responses that just need a status code
        Response::Stored => {
            let header = BinaryHeader::new_response(OPCODE_SET, 0, 0, STATUS_SUCCESS, 0, 0);
            header.write_to(&mut result)?;
        }
        Response::NotStored => {
            let header = BinaryHeader::new_response(OPCODE_SET, 0, 0, STATUS_ITEM_NOT_STORED, 0, 0);
            header.write_to(&mut result)?;
        }
        Response::Exists => {
            let header = BinaryHeader::new_response(OPCODE_SET, 0, 0, STATUS_KEY_EXISTS, 0, 0);
            header.write_to(&mut result)?;
        }
        Response::Deleted => {
            let header = BinaryHeader::new_response(OPCODE_DELETE, 0, 0, STATUS_SUCCESS, 0, 0);
            header.write_to(&mut result)?;
        }
        Response::Touched => {
            let header = BinaryHeader::new_response(OPCODE_NOOP, 0, 0, STATUS_SUCCESS, 0, 0);
            header.write_to(&mut result)?;
        }
        // Meta responses - not supported in binary protocol
        Response::MetaValue(_, _)
        | Response::MetaHit(_)
        | Response::MetaEnd
        | Response::MetaNoOp => {
            return Err(anyhow!("Meta commands not supported in binary protocol"));
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_header_serialization() {
        let header = BinaryHeader::new_request(OPCODE_GET, 5, 0, 5);
        let mut buffer = Vec::new();
        header.write_to(&mut buffer).unwrap();

        assert_eq!(buffer.len(), 24);
        assert_eq!(buffer[0], MAGIC_REQUEST);
        assert_eq!(buffer[1], OPCODE_GET);
    }

    #[test]
    fn test_parse_get_command() {
        let mut packet = Vec::new();
        let header = BinaryHeader::new_request(OPCODE_GET, 5, 0, 5);
        header.write_to(&mut packet).unwrap();
        packet.extend_from_slice(b"Hello");

        let (cmd, _opaque, _consumed) = parse_binary(&packet).unwrap();
        match cmd {
            Command::Get(keys) => {
                assert_eq!(keys.len(), 1);
                assert_eq!(keys[0], "Hello");
            }
            _ => panic!("Expected Get command"),
        }
    }

    #[test]
    fn test_parse_version_command() {
        let mut packet = Vec::new();
        let header = BinaryHeader::new_request(OPCODE_VERSION, 0, 0, 0);
        header.write_to(&mut packet).unwrap();

        let (cmd, _opaque, _consumed) = parse_binary(&packet).unwrap();
        assert!(matches!(cmd, Command::Version));
    }

    #[test]
    fn test_serialize_version_response() {
        let response = Response::Version("1.0.0".to_string());
        let data = serialize_binary_response(&response, 0).unwrap();

        assert!(data.len() >= 24); // At least header size
        assert_eq!(data[0], MAGIC_RESPONSE);
        assert_eq!(data[1], OPCODE_VERSION);
    }

    #[test]
    fn test_serialize_not_found_response() {
        let response = Response::NotFound;
        let data = serialize_binary_response(&response, 0).unwrap();

        assert_eq!(data.len(), 24); // Just header
        assert_eq!(data[0], MAGIC_RESPONSE);
        // Check status field (bytes 6-7)
        let status = u16::from_be_bytes([data[6], data[7]]);
        assert_eq!(status, STATUS_KEY_NOT_FOUND);
    }
}
