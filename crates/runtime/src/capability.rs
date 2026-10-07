
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::path::{Component, Path, PathBuf};

use crate::effect::{NotificationChannel, ProgramName};
use crate::{AgentHarnessName, PeerAdapterName, PeerAddress, PeerDisplayName, PeerRealmId};

mod sealed {
    pub trait Sealed {}

    pub trait CompleteAccessBoundary {}
}

pub trait EffectCapability: sealed::Sealed + 'static {
    const CAPABILITY: Capability;
}

macro_rules! define_effect_capabilities {
    ($($name:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub enum Capability {
            $($name),+
        }

        impl Capability {
            pub const COUNT: usize = [$(stringify!($name)),+].len();
            pub const ALL: [Self; Self::COUNT] = [$(Self::$name),+];

            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$name => stringify!($name)),+
                }
            }
        }

        impl ::std::fmt::Display for Capability {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        $(
            #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
            pub enum $name {}

            impl sealed::Sealed for $name {}

            impl EffectCapability for $name {
                const CAPABILITY: Capability = Capability::$name;
            }
        )+
    };
}

define_effect_capabilities! {
    HttpFetch,
    NetworkOutbound,
    NetworkListen,
    FsRead,
    FsWrite,
    ProcessSpawn,
    UserNotify,
    PeerConnect,
    ApprovalRequest,
    CivilTime,
    HostedDelegation,
    AgentHarness,
    ModelProvider,
    PeerDiscover,
    PeerSend,
    PeerAdvertise,
    PeerReceive,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Granted<C: EffectCapability>(PhantomData<fn() -> C>);

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GrantIssuer {
    _private: (),
}

#[cfg(any(test, feature = "test-support"))]
impl GrantIssuer {
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }

    #[must_use]
    pub const fn issue<C, P>(&self, _grant: &CapabilityGrant<C, P>) -> Granted<C>
    where
        C: EffectCapability,
    {
        Granted(PhantomData)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityGrant<C: EffectCapability, P> {
    parameters: P,
    capability: PhantomData<fn() -> C>,
}

impl<C: EffectCapability, P> CapabilityGrant<C, P> {
    #[must_use]
    pub const fn parameters(&self) -> &P {
        &self.parameters
    }

    #[must_use]
    pub fn into_parameters(self) -> P {
        self.parameters
    }

    #[must_use]
    pub fn evidence_for(&self, admitted: &CapabilitySet) -> Option<Granted<C>> {
        admitted
            .contains(C::CAPABILITY)
            .then_some(Granted(PhantomData))
    }
}

pub trait AgentHarnessAuthorityBearer {
    fn agent_harness_authority(&self) -> Option<Granted<AgentHarness>>;
}

pub trait FilesystemAuthorityBearer {
    fn fs_read_authority(&self) -> Option<Granted<FsRead>>;

    fn fs_write_authority(&self) -> Option<Granted<FsWrite>>;
}

pub trait ProcessAuthorityBearer {
    fn process_spawn_authority(&self) -> Option<Granted<ProcessSpawn>>;
}

pub trait UserNotifyAuthorityBearer {
    fn user_notify_authority(&self) -> Option<Granted<UserNotify>>;
}

pub trait HttpFetchAuthorityBearer {
    fn http_fetch_authority(&self) -> Option<Granted<HttpFetch>>;
}

pub trait PeerAuthorityBearer {
    fn peer_discover_authority(&self) -> Option<Granted<PeerDiscover>>;

    fn peer_send_authority(&self) -> Option<Granted<PeerSend>>;

    fn peer_advertise_authority(&self) -> Option<Granted<PeerAdvertise>>;

    fn peer_receive_authority(&self) -> Option<Granted<PeerReceive>>;
}

impl AgentHarnessAuthorityBearer for () {
    fn agent_harness_authority(&self) -> Option<Granted<AgentHarness>> {
        None
    }
}

impl FilesystemAuthorityBearer for () {
    fn fs_read_authority(&self) -> Option<Granted<FsRead>> {
        None
    }

    fn fs_write_authority(&self) -> Option<Granted<FsWrite>> {
        None
    }
}

impl ProcessAuthorityBearer for () {
    fn process_spawn_authority(&self) -> Option<Granted<ProcessSpawn>> {
        None
    }
}

impl UserNotifyAuthorityBearer for () {
    fn user_notify_authority(&self) -> Option<Granted<UserNotify>> {
        None
    }
}

impl HttpFetchAuthorityBearer for () {
    fn http_fetch_authority(&self) -> Option<Granted<HttpFetch>> {
        None
    }
}

impl PeerAuthorityBearer for () {
    fn peer_discover_authority(&self) -> Option<Granted<PeerDiscover>> {
        None
    }

    fn peer_send_authority(&self) -> Option<Granted<PeerSend>> {
        None
    }

    fn peer_advertise_authority(&self) -> Option<Granted<PeerAdvertise>> {
        None
    }

    fn peer_receive_authority(&self) -> Option<Granted<PeerReceive>> {
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathNormalizationError {
    NotAbsolute,
    EscapesRoot,
}

impl fmt::Display for PathNormalizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAbsolute => formatter.write_str("path is not absolute"),
            Self::EscapesRoot => formatter.write_str("path traverses above its root"),
        }
    }
}

impl Error for PathNormalizationError {}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NormalizedPath(PathBuf);

impl NormalizedPath {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, PathNormalizationError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(PathNormalizationError::NotAbsolute);
        }

        let mut normalized = PathBuf::new();
        let mut normal_components = 0_usize;
        for component in path.components() {
            match component {
                Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
                Component::RootDir => normalized.push(component.as_os_str()),
                Component::CurDir => {}
                Component::ParentDir => {
                    if normal_components == 0 {
                        return Err(PathNormalizationError::EscapesRoot);
                    }
                    normalized.pop();
                    normal_components -= 1;
                }
                Component::Normal(segment) => {
                    normalized.push(segment);
                    normal_components += 1;
                }
            }
        }
        Ok(Self(normalized))
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for NormalizedPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PathScope {
    root: NormalizedPath,
}

impl PathScope {
    #[must_use]
    pub const fn new(root: NormalizedPath) -> Self {
        Self { root }
    }

    #[must_use]
    pub const fn root(&self) -> &NormalizedPath {
        &self.root
    }

    #[must_use]
    pub fn contains(&self, candidate: &NormalizedPath) -> bool {
        candidate.as_path().starts_with(self.root.as_path())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PathScopes(BTreeSet<PathScope>);

impl PathScopes {
    #[must_use]
    pub fn new(scopes: impl IntoIterator<Item = PathScope>) -> Self {
        Self(scopes.into_iter().collect())
    }

    #[must_use]
    pub fn allows(&self, candidate: &NormalizedPath) -> bool {
        self.0.iter().any(|scope| scope.contains(candidate))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &PathScope> {
        self.0.iter()
    }
}

pub type FsReadGrant = CapabilityGrant<FsRead, PathScopes>;
pub type FsWriteGrant = CapabilityGrant<FsWrite, PathScopes>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentHarnessNames(BTreeSet<AgentHarnessName>);

impl AgentHarnessNames {
    #[must_use]
    pub fn new(names: impl IntoIterator<Item = AgentHarnessName>) -> Self {
        Self(names.into_iter().collect())
    }

    #[must_use]
    pub fn allows(&self, candidate: &AgentHarnessName) -> bool {
        self.0.contains(candidate)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &AgentHarnessName> {
        self.0.iter()
    }
}

pub type AgentHarnessGrant = CapabilityGrant<AgentHarness, AgentHarnessNames>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NotificationChannels(BTreeSet<NotificationChannel>);

impl NotificationChannels {
    #[must_use]
    pub fn exact(channels: impl IntoIterator<Item = NotificationChannel>) -> Self {
        Self(channels.into_iter().collect())
    }

    #[must_use]
    pub fn allows(&self, candidate: &NotificationChannel) -> bool {
        self.0.contains(candidate)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &NotificationChannel> {
        self.0.iter()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub type UserNotifyGrant = CapabilityGrant<UserNotify, NotificationChannels>;

impl CapabilityGrant<UserNotify, NotificationChannels> {
    #[must_use]
    pub const fn user_notify(channels: NotificationChannels) -> Self {
        Self {
            parameters: channels,
            capability: PhantomData,
        }
    }
}

/// One exact provider account/host/daemon boundary.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerRealmScope {
    adapter: PeerAdapterName,
    realm: PeerRealmId,
}

impl PeerRealmScope {
    #[must_use]
    pub const fn new(adapter: PeerAdapterName, realm: PeerRealmId) -> Self {
        Self { adapter, realm }
    }

    #[must_use]
    pub const fn adapter(&self) -> &PeerAdapterName {
        &self.adapter
    }

    #[must_use]
    pub const fn realm(&self) -> &PeerRealmId {
        &self.realm
    }

    #[must_use]
    pub fn contains(&self, address: &PeerAddress) -> bool {
        self.adapter == *address.adapter() && self.realm == *address.realm()
    }
}

/// Finite exact realm set used by discovery grants.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PeerRealmScopes(BTreeSet<PeerRealmScope>);

impl PeerRealmScopes {
    #[must_use]
    pub fn exact(scopes: impl IntoIterator<Item = PeerRealmScope>) -> Self {
        Self(scopes.into_iter().collect())
    }

    #[must_use]
    pub fn allows(&self, candidate: &PeerRealmScope) -> bool {
        self.0.contains(candidate)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &PeerRealmScope> {
        self.0.iter()
    }
}

/// Send authority over exact addresses or every address in one adapter realm.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PeerTargetScopes {
    exact: BTreeSet<PeerAddress>,
    realm: Option<PeerRealmScope>,
}

impl PeerTargetScopes {
    #[must_use]
    pub fn exact(addresses: impl IntoIterator<Item = PeerAddress>) -> Self {
        Self {
            exact: addresses.into_iter().collect(),
            realm: None,
        }
    }

    #[must_use]
    pub fn realm(adapter: PeerAdapterName, realm: PeerRealmId) -> Self {
        Self {
            exact: BTreeSet::new(),
            realm: Some(PeerRealmScope::new(adapter, realm)),
        }
    }

    #[must_use]
    pub fn allows(&self, candidate: &PeerAddress) -> bool {
        self.exact.contains(candidate)
            || self
                .realm
                .as_ref()
                .is_some_and(|realm| realm.contains(candidate))
    }

    /// Exact addresses only; a realm grant is represented by `realm_scope`.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &PeerAddress> {
        self.exact.iter()
    }

    #[must_use]
    pub const fn realm_scope(&self) -> Option<&PeerRealmScope> {
        self.realm.as_ref()
    }
}

/// One exact advertised name inside one exact provider realm.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerAdvertisementScope {
    realm: PeerRealmScope,
    name: PeerDisplayName,
}

impl PeerAdvertisementScope {
    #[must_use]
    pub const fn new(realm: PeerRealmScope, name: PeerDisplayName) -> Self {
        Self { realm, name }
    }

    #[must_use]
    pub const fn realm(&self) -> &PeerRealmScope {
        &self.realm
    }

    #[must_use]
    pub const fn name(&self) -> &PeerDisplayName {
        &self.name
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PeerAdvertisementScopes(BTreeSet<PeerAdvertisementScope>);

impl PeerAdvertisementScopes {
    /// Authorize one advertised name in one adapter realm.
    #[must_use]
    pub fn realm(adapter: PeerAdapterName, realm: PeerRealmId, name: PeerDisplayName) -> Self {
        Self::exact([PeerAdvertisementScope::new(
            PeerRealmScope::new(adapter, realm),
            name,
        )])
    }

    #[must_use]
    pub fn exact(scopes: impl IntoIterator<Item = PeerAdvertisementScope>) -> Self {
        Self(scopes.into_iter().collect())
    }

    #[must_use]
    pub fn allows(&self, candidate: &PeerAdvertisementScope) -> bool {
        self.0.contains(candidate)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &PeerAdvertisementScope> {
        self.0.iter()
    }
}

/// Receive authority over exact senders or every sender in one adapter realm.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PeerSenderScopes {
    exact: BTreeSet<PeerAddress>,
    realm: Option<PeerRealmScope>,
}

impl PeerSenderScopes {
    #[must_use]
    pub fn exact(addresses: impl IntoIterator<Item = PeerAddress>) -> Self {
        Self {
            exact: addresses.into_iter().collect(),
            realm: None,
        }
    }

    #[must_use]
    pub fn realm(adapter: PeerAdapterName, realm: PeerRealmId) -> Self {
        Self {
            exact: BTreeSet::new(),
            realm: Some(PeerRealmScope::new(adapter, realm)),
        }
    }

    #[must_use]
    pub fn allows(&self, candidate: &PeerAddress) -> bool {
        self.exact.contains(candidate)
            || self
                .realm
                .as_ref()
                .is_some_and(|realm| realm.contains(candidate))
    }

    /// Exact addresses only; a realm grant is represented by `realm_scope`.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &PeerAddress> {
        self.exact.iter()
    }

    #[must_use]
    pub const fn realm_scope(&self) -> Option<&PeerRealmScope> {
        self.realm.as_ref()
    }
}

pub type PeerDiscoverGrant = CapabilityGrant<PeerDiscover, PeerRealmScopes>;
pub type PeerSendGrant = CapabilityGrant<PeerSend, PeerTargetScopes>;
pub type PeerAdvertiseGrant = CapabilityGrant<PeerAdvertise, PeerAdvertisementScopes>;
pub type PeerReceiveGrant = CapabilityGrant<PeerReceive, PeerSenderScopes>;

impl From<PeerTargetScopes> for CapabilityGrant<PeerSend, PeerTargetScopes> {
    fn from(parameters: PeerTargetScopes) -> Self {
        Self {
            parameters,
            capability: PhantomData,
        }
    }
}

impl From<PeerAdvertisementScopes> for CapabilityGrant<PeerAdvertise, PeerAdvertisementScopes> {
    fn from(parameters: PeerAdvertisementScopes) -> Self {
        Self {
            parameters,
            capability: PhantomData,
        }
    }
}

impl From<PeerSenderScopes> for CapabilityGrant<PeerReceive, PeerSenderScopes> {
    fn from(parameters: PeerSenderScopes) -> Self {
        Self {
            parameters,
            capability: PhantomData,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessTargets(BTreeSet<ProgramName>);

impl ProcessTargets {
    #[must_use]
    pub fn exact(targets: impl IntoIterator<Item = ProgramName>) -> Self {
        Self(targets.into_iter().collect())
    }

    #[must_use]
    pub fn allows(&self, candidate: &ProgramName) -> bool {
        self.0.contains(candidate)
    }

    #[must_use]
    pub fn is_subset(&self, parent: &Self) -> bool {
        self.0.is_subset(&parent.0)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &ProgramName> {
        self.0.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[must_use]
pub fn authority_admits(entry: &str, authority: &str) -> bool {
    match entry.strip_suffix(":*") {
        Some(host) => authority_host(authority) == host,
        None => entry == authority,
    }
}

#[must_use]
pub fn authority_entry_is_valid(entry: &str) -> bool {
    let (authority, every_port) = match entry.strip_suffix(":*") {
        Some(host) => (host, true),
        None => (entry, false),
    };
    !authority.contains('*')
        && (!every_port || authority_host(authority) == authority)
        && crate::effect::validate_http_authority(authority).is_ok()
}

#[must_use]
pub fn authority_host(authority: &str) -> &str {
    if authority.starts_with('[') {
        return authority
            .find(']')
            .map_or(authority, |end| &authority[..=end]);
    }
    authority
        .rsplit_once(':')
        .map_or(authority, |(host, _)| host)
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HttpHosts(BTreeSet<Box<str>>);

impl HttpHosts {
    #[must_use]
    pub fn exact<H>(hosts: impl IntoIterator<Item = H>) -> Self
    where
        H: Into<Box<str>>,
    {
        Self(hosts.into_iter().map(Into::into).collect())
    }

    #[must_use]
    pub fn allows(&self, candidate: &str) -> bool {
        self.0
            .iter()
            .any(|entry| authority_admits(entry, candidate))
    }

    #[must_use]
    pub fn is_subset(&self, parent: &Self) -> bool {
        self.0.iter().all(|entry| parent.allows(entry))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &str> {
        self.0.iter().map(AsRef::as_ref)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub type HttpFetchGrant = CapabilityGrant<HttpFetch, HttpHosts>;

impl CapabilityGrant<HttpFetch, HttpHosts> {
    #[must_use]
    pub const fn http_fetch(hosts: HttpHosts) -> Self {
        Self {
            parameters: hosts,
            capability: PhantomData,
        }
    }
}

impl CapabilityGrant<FsRead, PathScopes> {
    #[must_use]
    pub const fn fs_read(scopes: PathScopes) -> Self {
        Self {
            parameters: scopes,
            capability: PhantomData,
        }
    }
}

impl CapabilityGrant<FsWrite, PathScopes> {
    #[must_use]
    pub const fn fs_write(scopes: PathScopes) -> Self {
        Self {
            parameters: scopes,
            capability: PhantomData,
        }
    }
}

impl CapabilityGrant<AgentHarness, AgentHarnessNames> {
    #[must_use]
    pub fn agent_harness(names: impl IntoIterator<Item = AgentHarnessName>) -> Self {
        Self {
            parameters: AgentHarnessNames::new(names),
            capability: PhantomData,
        }
    }
}

impl CapabilityGrant<PeerDiscover, PeerRealmScopes> {
    #[must_use]
    pub fn peer_discover(scopes: impl IntoIterator<Item = PeerRealmScope>) -> Self {
        Self {
            parameters: PeerRealmScopes::exact(scopes),
            capability: PhantomData,
        }
    }
}

impl CapabilityGrant<PeerSend, PeerTargetScopes> {
    #[must_use]
    pub fn peer_send(addresses: impl IntoIterator<Item = PeerAddress>) -> Self {
        Self {
            parameters: PeerTargetScopes::exact(addresses),
            capability: PhantomData,
        }
    }
}

impl CapabilityGrant<PeerAdvertise, PeerAdvertisementScopes> {
    #[must_use]
    pub fn peer_advertise(scopes: impl IntoIterator<Item = PeerAdvertisementScope>) -> Self {
        Self {
            parameters: PeerAdvertisementScopes::exact(scopes),
            capability: PhantomData,
        }
    }
}

impl CapabilityGrant<PeerReceive, PeerSenderScopes> {
    #[must_use]
    pub fn peer_receive(addresses: impl IntoIterator<Item = PeerAddress>) -> Self {
        Self {
            parameters: PeerSenderScopes::exact(addresses),
            capability: PhantomData,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Ceiling<U> {
    Unlimited,
    AtMost(U),
}

impl<U: Ord> Ceiling<U> {
    #[must_use]
    pub fn permits(&self, observed: &U) -> bool {
        match self {
            Self::Unlimited => true,
            Self::AtMost(limit) => observed <= limit,
        }
    }
}

macro_rules! resource_unit {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u64);

        impl $name {
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }
    };
}

resource_unit!(
    CpuTime,
    "Cumulative CPU time of the spawn and all its descendants (milliseconds)."
);
resource_unit!(
    MemoryBytes,
    "Resident memory of the spawn and all its descendants (bytes)."
);
resource_unit!(
    ProcessCount,
    "Number of spawn and descendant processes alive at once."
);

impl CpuTime {
    #[must_use]
    pub const fn from_millis(milliseconds: u64) -> Self {
        Self::new(milliseconds)
    }

    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.get()
    }
}

impl MemoryBytes {
    #[must_use]
    pub const fn from_bytes(bytes: u64) -> Self {
        Self::new(bytes)
    }

    #[must_use]
    pub const fn as_bytes(self) -> u64 {
        self.get()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResourceCeilings {
    cpu: Ceiling<CpuTime>,
    memory: Ceiling<MemoryBytes>,
    processes: Ceiling<ProcessCount>,
}

impl ResourceCeilings {
    #[must_use]
    pub const fn new(
        cpu: Ceiling<CpuTime>,
        memory: Ceiling<MemoryBytes>,
        processes: Ceiling<ProcessCount>,
    ) -> Self {
        Self {
            cpu,
            memory,
            processes,
        }
    }

    #[must_use]
    pub const fn cpu(&self) -> Ceiling<CpuTime> {
        self.cpu
    }

    #[must_use]
    pub const fn memory(&self) -> Ceiling<MemoryBytes> {
        self.memory
    }

    #[must_use]
    pub const fn processes(&self) -> Ceiling<ProcessCount> {
        self.processes
    }

    #[must_use]
    pub fn permits(&self, usage: &ResourceUsage) -> bool {
        self.cpu.permits(&usage.cpu)
            && self.memory.permits(&usage.memory)
            && self.processes.permits(&usage.processes)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResourceUsage {
    pub cpu: CpuTime,
    pub memory: MemoryBytes,
    pub processes: ProcessCount,
}

impl ResourceUsage {
    #[must_use]
    pub const fn new(cpu: CpuTime, memory: MemoryBytes, processes: ProcessCount) -> Self {
        Self {
            cpu,
            memory,
            processes,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AccessAllowance<S> {
    None,
    Allowed(S),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessBoundary<F, N, C, E> {
    files: AccessAllowance<F>,
    network: AccessAllowance<N>,
    children: AccessAllowance<C>,
    environment: AccessAllowance<E>,
}

impl<F, N, C, E> AccessBoundary<F, N, C, E> {
    #[must_use]
    pub const fn new(
        files: AccessAllowance<F>,
        network: AccessAllowance<N>,
        children: AccessAllowance<C>,
        environment: AccessAllowance<E>,
    ) -> Self {
        Self {
            files,
            network,
            children,
            environment,
        }
    }

    #[must_use]
    pub const fn files(&self) -> &AccessAllowance<F> {
        &self.files
    }

    #[must_use]
    pub const fn network(&self) -> &AccessAllowance<N> {
        &self.network
    }

    #[must_use]
    pub const fn children(&self) -> &AccessAllowance<C> {
        &self.children
    }

    #[must_use]
    pub const fn environment(&self) -> &AccessAllowance<E> {
        &self.environment
    }
}

impl<F, N, C, E> sealed::CompleteAccessBoundary for AccessBoundary<F, N, C, E> {}

pub trait CompleteAccessBoundary: sealed::CompleteAccessBoundary {}

impl<F, N, C, E> CompleteAccessBoundary for AccessBoundary<F, N, C, E> {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IsolationBoundary<A: CompleteAccessBoundary> {
    access: A,
    ceilings: ResourceCeilings,
}

impl<A: CompleteAccessBoundary> IsolationBoundary<A> {
    #[must_use]
    pub const fn new(access: A, ceilings: ResourceCeilings) -> Self {
        Self { access, ceilings }
    }

    #[must_use]
    pub const fn access(&self) -> &A {
        &self.access
    }

    #[must_use]
    pub const fn ceilings(&self) -> &ResourceCeilings {
        &self.ceilings
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSpawnParameters<T, A: CompleteAccessBoundary> {
    targets: T,
    isolation: IsolationBoundary<A>,
}

impl<T, A: CompleteAccessBoundary> ProcessSpawnParameters<T, A> {
    #[must_use]
    pub const fn new(targets: T, isolation: IsolationBoundary<A>) -> Self {
        Self { targets, isolation }
    }

    #[must_use]
    pub const fn targets(&self) -> &T {
        &self.targets
    }

    #[must_use]
    pub const fn isolation(&self) -> &IsolationBoundary<A> {
        &self.isolation
    }
}

pub type ProcessSpawnGrant<T, A> = CapabilityGrant<ProcessSpawn, ProcessSpawnParameters<T, A>>;

pub type WorkspaceProcessAccess = AccessBoundary<PathScopes, (), (), ()>;
pub type WorkspaceProcessGrant = ProcessSpawnGrant<ProcessTargets, WorkspaceProcessAccess>;

impl CapabilityGrant<ProcessSpawn, ProcessSpawnParameters<ProcessTargets, WorkspaceProcessAccess>> {
    #[must_use]
    pub fn workspace_process(
        targets: ProcessTargets,
        files: PathScopes,
        ceilings: ResourceCeilings,
    ) -> Self {
        Self::process_spawn(
            targets,
            IsolationBoundary::new(
                AccessBoundary::new(
                    AccessAllowance::Allowed(files),
                    AccessAllowance::None,
                    AccessAllowance::None,
                    AccessAllowance::None,
                ),
                ceilings,
            ),
        )
    }
}

impl<T, A: CompleteAccessBoundary> CapabilityGrant<ProcessSpawn, ProcessSpawnParameters<T, A>> {
    #[must_use]
    pub const fn process_spawn(targets: T, isolation: IsolationBoundary<A>) -> Self {
        Self {
            parameters: ProcessSpawnParameters::new(targets, isolation),
            capability: PhantomData,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilitySet(BTreeSet<Capability>);

impl CapabilitySet {
    #[must_use]
    pub const fn empty() -> Self {
        Self(BTreeSet::new())
    }

    #[must_use]
    pub fn singleton(capability: Capability) -> Self {
        Self(BTreeSet::from([capability]))
    }

    #[must_use]
    pub fn join(&self, other: &Self) -> Self {
        Self(self.0.union(&other.0).copied().collect())
    }

    #[must_use]
    pub fn contains(&self, capability: Capability) -> bool {
        self.0.contains(&capability)
    }

    #[must_use]
    pub fn is_subset(&self, other: &Self) -> bool {
        self.0.is_subset(&other.0)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = Capability> + '_ {
        self.0.iter().copied()
    }
}

