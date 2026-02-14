use std::fs;
use std::path::Path;

pub const PYRS_EXTENSION_MANIFEST_SUFFIX: &str = ".pyrs-ext";
pub const PYRS_EXTENSION_ABI_TAG: &str = "pyrs314";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionEntrypoint {
    HelloExt,
}

impl ExtensionEntrypoint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HelloExt => "hello_ext",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "hello_ext" => Some(Self::HelloExt),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionManifest {
    pub module_name: String,
    pub abi_tag: String,
    pub entrypoint: ExtensionEntrypoint,
}

pub fn parse_extension_manifest(
    path: &Path,
    expected_module_name: &str,
) -> Result<ExtensionManifest, String> {
    let raw = fs::read_to_string(path).map_err(|err| {
        format!(
            "failed to read extension manifest '{}': {err}",
            path.display()
        )
    })?;
    let mut module_name: Option<String> = None;
    let mut abi_tag: Option<String> = None;
    let mut entrypoint: Option<ExtensionEntrypoint> = None;

    for (line_number, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            return Err(format!(
                "invalid manifest line {} in '{}': expected key=value",
                line_number + 1,
                path.display()
            ));
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "module" => module_name = Some(value.to_string()),
            "abi" => abi_tag = Some(value.to_string()),
            "entrypoint" => {
                let parsed = ExtensionEntrypoint::parse(value).ok_or_else(|| {
                    format!(
                        "unsupported extension entrypoint '{}' in '{}'",
                        value,
                        path.display()
                    )
                })?;
                entrypoint = Some(parsed);
            }
            other => {
                return Err(format!(
                    "unsupported manifest key '{}' in '{}'",
                    other,
                    path.display()
                ));
            }
        }
    }

    let module_name = module_name.ok_or_else(|| {
        format!(
            "missing required 'module' key in extension manifest '{}'",
            path.display()
        )
    })?;
    if module_name != expected_module_name {
        return Err(format!(
            "manifest module '{}' does not match import target '{}'",
            module_name, expected_module_name
        ));
    }

    let abi_tag = abi_tag.ok_or_else(|| {
        format!(
            "missing required 'abi' key in extension manifest '{}'",
            path.display()
        )
    })?;
    if abi_tag != PYRS_EXTENSION_ABI_TAG {
        return Err(format!(
            "unsupported extension ABI '{}'; expected '{}'",
            abi_tag, PYRS_EXTENSION_ABI_TAG
        ));
    }

    let entrypoint = entrypoint.ok_or_else(|| {
        format!(
            "missing required 'entrypoint' key in extension manifest '{}'",
            path.display()
        )
    })?;

    Ok(ExtensionManifest {
        module_name,
        abi_tag,
        entrypoint,
    })
}

#[cfg(test)]
mod tests {
    use super::{ExtensionEntrypoint, PYRS_EXTENSION_ABI_TAG, parse_extension_manifest};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_manifest_path(stem: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("pyrs_manifest_{stem}_{nanos}.pyrs-ext"))
    }

    #[test]
    fn parses_valid_manifest() {
        let path = temp_manifest_path("valid");
        fs::write(
            &path,
            format!("module=hello_ext\nabi={PYRS_EXTENSION_ABI_TAG}\nentrypoint=hello_ext\n"),
        )
        .expect("manifest write should succeed");

        let parsed = parse_extension_manifest(&path, "hello_ext").expect("manifest should parse");
        assert_eq!(parsed.module_name, "hello_ext");
        assert_eq!(parsed.abi_tag, PYRS_EXTENSION_ABI_TAG);
        assert_eq!(parsed.entrypoint, ExtensionEntrypoint::HelloExt);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_unexpected_module_name() {
        let path = temp_manifest_path("module_mismatch");
        fs::write(
            &path,
            format!("module=other\nabi={PYRS_EXTENSION_ABI_TAG}\nentrypoint=hello_ext\n"),
        )
        .expect("manifest write should succeed");

        let err = parse_extension_manifest(&path, "hello_ext").expect_err("manifest should fail");
        assert!(err.contains("does not match import target"));

        let _ = fs::remove_file(path);
    }
}
