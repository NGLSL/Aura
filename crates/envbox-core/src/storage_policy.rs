//! Storage configuration and lexical preview. Final object authorization belongs to a backend.
use crate::DomainError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageAction {
    IsolatedWrite,
    SharedReadOnly,
    SharedReadWrite,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageTarget {
    FileDirectory,
    RegistrySubtree,
}

impl std::fmt::Display for StorageAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::IsolatedWrite => "隔离写入",
            Self::SharedReadOnly => "共享只读",
            Self::SharedReadWrite => "共享可写（修改宿主）",
            Self::Deny => "拒绝",
        })
    }
}
impl std::fmt::Display for StorageTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FileDirectory => "应用文件目录",
            Self::RegistrySubtree => "HKCU 应用子树",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageRule {
    pub target: StorageTarget,
    pub path: String,
    pub action: StorageAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoragePolicy {
    pub schema_version: u32,
    pub rules: Vec<StorageRule>,
}
impl Default for StoragePolicy {
    fn default() -> Self {
        Self {
            schema_version: 1,
            rules: vec![],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoragePreview {
    pub configured_action: Option<StorageAction>,
    pub matched_rule: Option<String>,
    pub host_write_exception: bool,
    /// Always false until the final object, volume and reparse/alias identity are proven.
    pub can_authorize: bool,
}

fn invalid(reason: impl Into<String>) -> DomainError {
    DomainError::InvalidContainer(format!("storage policy: {}", reason.into()))
}
fn within(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

pub fn normalize_storage_path(target: StorageTarget, path: &str) -> Result<String, DomainError> {
    if path.is_empty() || path.chars().any(char::is_control) {
        return Err(invalid("empty or control-containing path"));
    }
    let value = path.replace('/', "\\");
    let components: Vec<&str> = value.split('\\').collect();
    match target {
        StorageTarget::FileDirectory => {
            if components.len() < 2
                || components[0].len() != 2
                || !components[0].as_bytes()[0].is_ascii_alphabetic()
                || components[0].as_bytes()[1] != b':'
            {
                return Err(invalid("only absolute local drive paths are supported; UNC and device aliases require a backend"));
            }
            if components[1].is_empty() && components.len() == 2 {
                return Err(invalid("a whole drive is not an application directory"));
            }
        }
        StorageTarget::RegistrySubtree => {
            if components.len() < 4
                || !components[0].eq_ignore_ascii_case("HKCU")
                || !components[1].eq_ignore_ascii_case("Software")
            {
                return Err(invalid(
                    "only explicit HKCU\\Software\\Vendor\\Application subtrees are supported",
                ));
            }
            if ["Classes", "Policies", "Aura"]
                .iter()
                .any(|name| components[2].eq_ignore_ascii_case(name))
            {
                return Err(invalid(
                    "shared system and Aura management Registry subtrees are reserved",
                ));
            }
        }
    }
    let mut output = vec![components[0].to_ascii_lowercase()];
    for (index, component) in components.iter().enumerate().skip(1) {
        if component.is_empty() && index == components.len() - 1 {
            continue;
        }
        if component.is_empty()
            || *component == "."
            || *component == ".."
            || component.ends_with([' ', '.'])
            || component.contains([':', '*', '?', '"', '<', '>', '|'])
        {
            return Err(invalid(
                "ambiguous, relative, stream or invalid path component",
            ));
        }
        if target == StorageTarget::FileDirectory {
            let stem = component
                .split('.')
                .next()
                .unwrap_or("")
                .to_ascii_uppercase();
            if ["CON", "PRN", "AUX", "NUL", "CLOCK$"].contains(&stem.as_str())
                || (stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit())
            {
                return Err(invalid("reserved DOS device name"));
            }
        }
        output.push(component.to_ascii_lowercase());
    }
    if target == StorageTarget::FileDirectory && output.len() < 2 {
        return Err(invalid("a whole drive is not an application directory"));
    }
    Ok(output.join("\\"))
}

impl StoragePolicy {
    pub fn validate(&self, management_root: &str) -> Result<(), DomainError> {
        if self.schema_version != 1 {
            return Err(invalid(format!(
                "unsupported schema {}",
                self.schema_version
            )));
        }
        let root = if self
            .rules
            .iter()
            .any(|rule| rule.target == StorageTarget::FileDirectory)
        {
            Some(normalize_storage_path(
                StorageTarget::FileDirectory,
                management_root,
            )?)
        } else {
            None
        };
        let mut identities = HashMap::new();
        for rule in &self.rules {
            let normalized = normalize_storage_path(rule.target, &rule.path)?;
            if let Some(root) = &root {
                if rule.target == StorageTarget::FileDirectory
                    && (within(&normalized, root) || within(root, &normalized))
                {
                    return Err(invalid(
                        "Aura management and Overlay roots cannot be selected or shared",
                    ));
                }
            }
            if let Some(previous) = identities.insert((rule.target, normalized), rule.action) {
                if previous != rule.action {
                    return Err(invalid("equivalent lexical paths have conflicting actions"));
                }
                return Err(invalid("duplicate equivalent lexical rule"));
            }
        }
        Ok(())
    }

    pub fn preview(
        &self,
        target: StorageTarget,
        path: &str,
        management_root: &str,
    ) -> Result<StoragePreview, DomainError> {
        self.validate(management_root)?;
        let path = normalize_storage_path(target, path)?;
        if target == StorageTarget::FileDirectory {
            let root = normalize_storage_path(target, management_root)?;
            if within(&path, &root) || within(&root, &path) {
                return Err(invalid("direct management/Overlay access is denied"));
            }
        }
        let mut chosen: Option<(usize, &StorageRule)> = None;
        for rule in &self.rules {
            if rule.target != target {
                continue;
            }
            let root = normalize_storage_path(target, &rule.path)?;
            if within(&path, &root) && chosen.is_none_or(|(length, _)| root.len() > length) {
                chosen = Some((root.len(), rule));
            }
        }
        let configured_action = chosen.map(|(_, rule)| rule.action);
        Ok(StoragePreview {
            configured_action,
            matched_rule: chosen.map(|(_, rule)| rule.path.clone()),
            host_write_exception: configured_action == Some(StorageAction::SharedReadWrite),
            can_authorize: false,
        })
    }
}
