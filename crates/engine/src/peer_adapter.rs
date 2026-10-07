
use crate::execution_profile::{BUILT_IN_PEER_ADAPTER, PeerAdapterFactory};
use crate::harness_adapter::{DeclarationError, Section, error};
use circular_runtime::PeerAdapterName;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use toml_edit::DocumentMut;

pub type PeerBridgeConstructor =
    fn(PeerAdapterName, &PeerSettings) -> Result<PeerAdapterFactory, String>;

const DECLARATIONS: [(&str, &str, PeerBridgeConstructor); 2] = [
    (
        "claude.toml",
        include_str!("../peer-adapters/claude.toml"),
        crate::peer_bridges::claude::declared_bridge,
    ),
    (
        "codex.toml",
        include_str!("../peer-adapters/codex.toml"),
        crate::peer_bridges::codex::declared_bridge,
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingKind {
    Path,
    Text,
}

#[derive(Debug, Eq, PartialEq)]
struct DeclaredSetting {
    key: String,
    kind: SettingKind,
    default: String,
}

pub struct PeerAdapterDeclaration {
    name: PeerAdapterName,
    settings: Vec<DeclaredSetting>,
    bridge: PeerBridgeConstructor,
}

impl std::fmt::Debug for PeerAdapterDeclaration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerAdapterDeclaration")
            .field("name", &self.name)
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl PartialEq for PeerAdapterDeclaration {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for PeerAdapterDeclaration {}

impl PeerAdapterDeclaration {
    fn parse(source: &str, bridge: PeerBridgeConstructor) -> Result<Self, DeclarationError> {
        let document = source
            .parse::<DocumentMut>()
            .map_err(|failure| error(format!("not TOML: {failure}")))?;
        let root = Section::of(document.as_item(), "peer adapter", &["name", "settings"])?;
        let name = root.string("name")?;
        let name = PeerAdapterName::try_new(name.as_str())
            .map_err(|failure| error(format!("name {name:?}: {failure}")))?;
        let settings = match root.get("settings") {
            None => Vec::new(),
            Some(item) => item
                .as_table_like()
                .ok_or_else(|| error("settings must be a table"))?
                .iter()
                .map(|(key, item)| setting(key, item))
                .collect::<Result<_, _>>()?,
        };
        Ok(Self {
            name,
            settings,
            bridge,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn setting_keys(&self) -> impl Iterator<Item = &str> {
        self.settings.iter().map(|setting| setting.key.as_str())
    }

    pub fn clause<E>(
        &'static self,
        mut authored: impl FnMut(&str) -> Result<Option<Box<str>>, E>,
    ) -> Result<PeerAdapterClause, E> {
        let values = self
            .settings
            .iter()
            .map(|setting| {
                let value = authored(&setting.key)?;
                Ok((
                    setting,
                    value.unwrap_or_else(|| setting.default.as_str().into()),
                ))
            })
            .collect::<Result<_, E>>()?;
        Ok(PeerAdapterClause {
            declaration: self,
            values,
        })
    }
}

fn setting(key: &str, item: &toml_edit::Item) -> Result<DeclaredSetting, DeclarationError> {
    if key == "name" {
        return Err(error("settings.name would shadow the clause's own name"));
    }
    let entry = Section::of(item, format!("settings.{key}"), &["path", "text"])?;
    let (kind, field) = match (entry.get("path"), entry.get("text")) {
        (Some(_), None) => (SettingKind::Path, "path"),
        (None, Some(_)) => (SettingKind::Text, "text"),
        _ => {
            return Err(error(format!(
                "settings.{key} must hold exactly one of path or text"
            )));
        }
    };
    Ok(DeclaredSetting {
        key: key.to_owned(),
        kind,
        default: entry.string(field)?,
    })
}

pub fn declared_peer_adapters() -> &'static [PeerAdapterDeclaration] {
    static DECLARED: OnceLock<Vec<PeerAdapterDeclaration>> = OnceLock::new();
    DECLARED.get_or_init(|| {
        let adapters = DECLARATIONS
            .iter()
            .map(|(file, source, bridge)| {
                PeerAdapterDeclaration::parse(source, *bridge)
                    .unwrap_or_else(|failure| panic!("peer adapter declaration {file}: {failure}"))
            })
            .collect::<Vec<_>>();
        for (index, adapter) in adapters.iter().enumerate() {
            assert!(
                adapter.name() != BUILT_IN_PEER_ADAPTER
                    && adapters[..index].iter().all(|earlier| earlier != adapter),
                "peer adapter declaration name {:?} is declared twice or shadows the built-in adapter",
                adapter.name()
            );
        }
        adapters
    })
}

pub fn selectable_peer_adapter_names() -> impl Iterator<Item = &'static str> {
    std::iter::once(BUILT_IN_PEER_ADAPTER).chain(
        declared_peer_adapters()
            .iter()
            .map(PeerAdapterDeclaration::name),
    )
}

#[must_use]
pub fn peer_adapter_declaration(name: &str) -> Option<&'static PeerAdapterDeclaration> {
    declared_peer_adapters()
        .iter()
        .find(|declaration| declaration.name() == name)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerAdapterClause {
    declaration: &'static PeerAdapterDeclaration,
    values: Vec<(&'static DeclaredSetting, Box<str>)>,
}

impl PeerAdapterClause {
    #[must_use]
    pub fn name(&self) -> &str {
        self.declaration.name()
    }

    pub fn values(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values
            .iter()
            .map(|(setting, value)| (setting.key.as_str(), value.as_ref()))
    }

    #[must_use]
    pub fn resolve(&self, home: &Path) -> ResolvedPeerAdapter {
        let settings = self
            .values
            .iter()
            .map(|(setting, value)| {
                let value = match setting.kind {
                    SettingKind::Path => SettingValue::Path(expand_home(value, home)),
                    SettingKind::Text => SettingValue::Text(value.to_string()),
                };
                (setting.key.clone(), value)
            })
            .collect();
        ResolvedPeerAdapter {
            declaration: self.declaration,
            settings: PeerSettings(settings),
        }
    }
}

#[derive(Debug)]
pub struct ResolvedPeerAdapter {
    declaration: &'static PeerAdapterDeclaration,
    settings: PeerSettings,
}

impl ResolvedPeerAdapter {
    #[must_use]
    pub fn name(&self) -> &PeerAdapterName {
        &self.declaration.name
    }

    #[must_use]
    pub fn settings(&self) -> &PeerSettings {
        &self.settings
    }

    pub fn factory(&self) -> Result<PeerAdapterFactory, String> {
        (self.declaration.bridge)(self.declaration.name.clone(), &self.settings)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerSettings(Vec<(String, SettingValue)>);

#[derive(Clone, Debug, Eq, PartialEq)]
enum SettingValue {
    Path(PathBuf),
    Text(String),
}

impl PeerSettings {
    fn value(&self, key: &str) -> &SettingValue {
        self.0
            .iter()
            .find_map(|(declared, value)| (declared == key).then_some(value))
            .unwrap_or_else(|| {
                panic!("the bridge reads setting {key:?}, which its declaration does not declare")
            })
    }

    #[must_use]
    pub fn path(&self, key: &str) -> &Path {
        match self.value(key) {
            SettingValue::Path(path) => path,
            SettingValue::Text(_) => panic!("setting {key:?} is declared as text, not path"),
        }
    }

    #[must_use]
    pub fn text(&self, key: &str) -> &str {
        match self.value(key) {
            SettingValue::Text(text) => text,
            SettingValue::Path(_) => panic!("setting {key:?} is declared as path, not text"),
        }
    }
}

impl std::fmt::Display for PeerSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, (key, value)) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            match value {
                SettingValue::Path(path) => write!(formatter, "{key}={}", path.display())?,
                SettingValue::Text(text) => write!(formatter, "{key}={text}")?,
            }
        }
        Ok(())
    }
}

fn expand_home(value: &str, home: &Path) -> PathBuf {
    match value.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if value == "~" => home.to_path_buf(),
        None => PathBuf::from(value),
    }
}
