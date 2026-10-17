use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;

/// Versioned API family. An API range is a compatibility declaration, not a grant.
pub const CURRENT_API_NAME: &str = "akula.plugin";
pub const CURRENT_API_LEVEL: u16 = 1;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub schema: u16,
    pub id: String,
    /// Self-declared publisher namespace; the installer must verify publisher identity.
    pub publisher: String,
    pub display_name: String,
    pub version: String,
    pub api: ApiRequirement,
    pub runtime: PluginRuntime,
    /// Relative WASM entry inside an untrusted package; never an OS path.
    pub entry: String,
    pub platforms: Vec<HostPlatform>,
    #[serde(default)]
    pub dependencies: Vec<PluginDependency>,
    #[serde(default)]
    pub permissions: PermissionRequests,
    /// HTTPS destination hostnames requested by a future network broker.
    /// These are declarations, not an allowance to perform network I/O.
    #[serde(default)]
    pub network_domains: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiRequirement {
    pub family: String,
    pub min: u16,
    pub max: u16,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginRuntime {
    WasmComponent,
    WasmModule,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostPlatform {
    Windows,
    Linux,
    Macos,
}

/// A closed set of requestable capabilities. In particular, there is no shell,
/// raw filesystem, FFI, env, arbitrary TCP/UDP or implicit provider-token grant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum Capability {
    #[serde(rename = "ui.command")]
    UiCommand,
    #[serde(rename = "ui.panel")]
    UiPanel,
    #[serde(rename = "notes.read")]
    NotesRead,
    #[serde(rename = "notes.search")]
    NotesSearch,
    #[serde(rename = "notes.patch")]
    NotesPatch,
    #[serde(rename = "notes.create")]
    NotesCreate,
    #[serde(rename = "notes.trash")]
    NotesTrash,
    #[serde(rename = "attachments.read")]
    AttachmentsRead,
    #[serde(rename = "attachments.write")]
    AttachmentsWrite,
    #[serde(rename = "events.subscribe")]
    EventsSubscribe,
    #[serde(rename = "network.fetch")]
    NetworkFetch,
    #[serde(rename = "clipboard.read")]
    ClipboardRead,
    #[serde(rename = "clipboard.write")]
    ClipboardWrite,
    #[serde(rename = "ai.invoke")]
    AiInvoke,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionRequests {
    #[serde(default)]
    pub required: Vec<Capability>,
    #[serde(default)]
    pub optional: Vec<Capability>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDependency {
    pub id: String,
    pub min_version: String,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ManifestError {
    InvalidJson(String),
    Invalid(&'static str),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(message) => write!(formatter, "invalid plugin JSON: {message}"),
            Self::Invalid(message) => write!(formatter, "invalid plugin manifest: {message}"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl PluginManifest {
    /// Parse untrusted JSON without any filesystem, network or plugin side effect.
    pub fn parse_json(input: &str) -> Result<Self, ManifestError> {
        if input.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::Invalid("manifest exceeds 64 KiB"));
        }
        let manifest: Self =
            serde_json::from_str(input).map_err(|e| ManifestError::InvalidJson(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Reject ambiguous or unsupported declarations before any future install.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema != 1 {
            return Err(ManifestError::Invalid("unsupported manifest schema"));
        }
        if !valid_namespace(&self.publisher, 2) || !valid_namespace(&self.id, 3) {
            return Err(ManifestError::Invalid("invalid reverse-DNS identity"));
        }
        if !self.id.starts_with(&(self.publisher.clone() + ".")) {
            return Err(ManifestError::Invalid("plugin ID is outside publisher namespace"));
        }
        if self.display_name.trim().is_empty()
            || self.display_name.len() > 120
            || self.display_name.chars().any(char::is_control)
        {
            return Err(ManifestError::Invalid("invalid display name"));
        }
        if !valid_numeric_version(&self.version) {
            return Err(ManifestError::Invalid("invalid release version"));
        }
        if self.api.family != CURRENT_API_NAME
            || self.api.min == 0
            || self.api.min > self.api.max
            || !(self.api.min..=self.api.max).contains(&CURRENT_API_LEVEL)
        {
            return Err(ManifestError::Invalid("incompatible plugin API range"));
        }
        if !valid_entry(&self.entry) {
            return Err(ManifestError::Invalid("invalid WASM package entry"));
        }

        if self.platforms.is_empty() || self.platforms.len() > 3 {
            return Err(ManifestError::Invalid("invalid platform count"));
        }
        let mut seen_platforms = HashSet::new();
        for platform in &self.platforms {
            if !seen_platforms.insert(*platform) {
                return Err(ManifestError::Invalid("duplicate platform"));
            }
        }

        let mut seen_permissions = HashSet::new();
        for permission in self
            .permissions
            .required
            .iter()
            .chain(self.permissions.optional.iter())
        {
            if !seen_permissions.insert(*permission) {
                return Err(ManifestError::Invalid("duplicate or overlapping capability"));
            }
        }
        if self.dependencies.len() > 32 {
            return Err(ManifestError::Invalid("too many dependencies"));
        }
        let mut seen_dependencies = HashSet::new();
        for dependency in &self.dependencies {
            if !valid_namespace(&dependency.id, 3)
                || dependency.id == self.id
                || !valid_numeric_version(&dependency.min_version)
            {
                return Err(ManifestError::Invalid("invalid plugin dependency"));
            }
            if !seen_dependencies.insert(dependency.id.as_str()) {
                return Err(ManifestError::Invalid("duplicate dependency"));
            }
        }

        if self.network_domains.len() > 32 {
            return Err(ManifestError::Invalid("too many network domains"));
        }
        let mut seen_domains = HashSet::new();
        for domain in &self.network_domains {
            if !valid_domain(domain) || !seen_domains.insert(domain.as_str()) {
                return Err(ManifestError::Invalid("invalid or duplicate network domain"));
            }
        }
        let wants_network = seen_permissions.contains(&Capability::NetworkFetch);
        if wants_network != !self.network_domains.is_empty() {
            return Err(ManifestError::Invalid(
                "network.fetch must declare one or more restricted domains",
            ));
        }

        Ok(())
    }

    pub fn to_json_pretty(&self) -> Result<String, ManifestError> {
        self.validate()?;
        serde_json::to_string_pretty(self)
            .map_err(|e| ManifestError::InvalidJson(e.to_string()))
    }
}

fn valid_namespace(value: &str, min_segments: usize) -> bool {
    if value.len() > 128 {
        return false;
    }
    let segments: Vec<&str> = value.split('.').collect();
    segments.len() >= min_segments
        && segments.iter().all(|segment| {
            let bytes = segment.as_bytes();
            !bytes.is_empty()
                && bytes.len() <= 63
                && bytes[0].is_ascii_lowercase()
                && bytes[bytes.len() - 1].is_ascii_alphanumeric()
                && bytes
                    .iter()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == b'-')
        })
}

fn valid_numeric_version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && (p.len() == 1 || !p.starts_with('0'))
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u32>().is_ok()
        })
}

fn valid_entry(entry: &str) -> bool {
    if entry.len() > 240 || !entry.ends_with(".wasm") || entry.starts_with('/') {
        return false;
    }
    entry.split('/').all(|segment| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment.bytes().all(|b| {
                b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || b == b'-'
                    || b == b'_'
                    || b == b'.'
            })
    })
}

fn valid_domain(domain: &str) -> bool {
    if domain.len() > 253
        || domain.ends_with(".local")
        || domain.ends_with(".internal")
        || domain.ends_with(".localhost")
    {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    labels.len() >= 2
        && !labels.last().is_some_and(|last| last.bytes().all(|b| b.is_ascii_digit()))
        && labels.iter().all(|label| {
            let bytes = label.as_bytes();
            !bytes.is_empty()
                && bytes.len() <= 63
                && bytes[0].is_ascii_alphanumeric()
                && bytes[bytes.len() - 1].is_ascii_alphanumeric()
                && bytes
                    .iter()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        })
}
