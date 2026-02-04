#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PycHeader {
    pub magic: u32,
    pub bitfield: u32,
    pub timestamp: Option<u32>,
    pub source_size: Option<u32>,
    pub hash: Option<[u8; 8]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PycError {
    pub message: String,
}

impl PycError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Parse the CPython 3.14 .pyc header. Returns the header and the offset
/// where the marshaled code object begins.
pub fn parse_pyc_header(bytes: &[u8]) -> Result<(PycHeader, usize), PycError> {
    if bytes.len() < 8 {
        return Err(PycError::new("pyc header is too short"));
    }

    let magic = read_u32_le(bytes, 0)?;
    let bitfield = read_u32_le(bytes, 4)?;
    let mut offset = 8;

    if bitfield & 0x01 != 0 {
        if bytes.len() < offset + 8 {
            return Err(PycError::new("pyc hash header is too short"));
        }
        let hash: [u8; 8] = bytes[offset..offset + 8]
            .try_into()
            .map_err(|_| PycError::new("invalid hash length"))?;
        offset += 8;
        Ok((
            PycHeader {
                magic,
                bitfield,
                timestamp: None,
                source_size: None,
                hash: Some(hash),
            },
            offset,
        ))
    } else {
        if bytes.len() < offset + 8 {
            return Err(PycError::new("pyc timestamp header is too short"));
        }
        let timestamp = read_u32_le(bytes, offset)?;
        let source_size = read_u32_le(bytes, offset + 4)?;
        offset += 8;
        Ok((
            PycHeader {
                magic,
                bitfield,
                timestamp: Some(timestamp),
                source_size: Some(source_size),
                hash: None,
            },
            offset,
        ))
    }
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, PycError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| PycError::new("unexpected end of file"))?;
    Ok(u32::from_le_bytes(
        slice.try_into().map_err(|_| PycError::new("invalid u32"))?,
    ))
}
