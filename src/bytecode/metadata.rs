use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpcodeInfo {
    pub code: u16,
    pub name: String,
    pub stack_effect: i16,
    pub flags: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpcodeMetadata {
    pub opcodes: Vec<OpcodeInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataError {
    pub message: String,
}

impl MetadataError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl OpcodeMetadata {
    pub fn empty() -> Self {
        Self {
            opcodes: Vec::new(),
        }
    }

    pub fn load_default() -> Result<Self, MetadataError> {
        let path = vendor_dir().join("opcode_table.csv");
        if !path.exists() {
            return Ok(Self::empty());
        }
        Self::load_from_csv(&path)
    }

    pub fn load_from_csv(path: &Path) -> Result<Self, MetadataError> {
        let data = fs::read_to_string(path)
            .map_err(|err| MetadataError::new(format!("failed to read {path:?}: {err}")))?;

        let mut opcodes = Vec::new();
        for (line_no, line) in data.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line_no == 0 && line.to_lowercase().starts_with("opcode") {
                continue;
            }

            let parts: Vec<&str> = line.split(',').map(|part| part.trim()).collect();
            if parts.len() < 3 {
                return Err(MetadataError::new(format!(
                    "invalid opcode row at line {}",
                    line_no + 1
                )));
            }

            let code = parts[0].parse::<u16>().map_err(|_| {
                MetadataError::new(format!("invalid opcode number at line {}", line_no + 1))
            })?;
            let name = parts[1].to_string();
            let stack_effect = parts[2].parse::<i16>().map_err(|_| {
                MetadataError::new(format!("invalid stack effect at line {}", line_no + 1))
            })?;
            let flags = if parts.len() > 3 {
                parts[3].to_string()
            } else {
                String::new()
            };

            opcodes.push(OpcodeInfo {
                code,
                name,
                stack_effect,
                flags,
            });
        }

        Ok(Self { opcodes })
    }
}

fn vendor_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/cpython-3.14/opcode")
}
