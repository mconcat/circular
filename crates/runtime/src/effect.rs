
use crate::capability::{
    AgentHarness, ApprovalRequest, Capability, CapabilitySet, FsRead, FsWrite, Granted, HttpFetch,
    ProcessSpawn, UserNotify,
};
use crate::reason::{DeclaredReason, SuppressionReason};
use crate::{AgentInvokeSpec, ApprovalSpec, ApprovalTicket, NormalizedPath};
use circular_core::NonZeroMillis;
use circular_plan::PortId;

macro_rules! define_effect_ctors {
    ($($name:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub enum EffectCtor {
            $($name),+
        }

        impl EffectCtor {
            pub const COUNT: usize = [$(stringify!($name)),+].len();
            pub const ALL: [Self; Self::COUNT] = [$(Self::$name),+];
        }
    };
}

define_effect_ctors! {
    Http,
    FileRead,
    FileWrite,
    Spawn,
    Notify,
    Publish,
    Listen,
    Receive,
    RequestApproval,
    CivilResolve,
    HostedInvoke,
    AgentInvoke,
    ModelInvoke,
    PeerDiscover,
    PeerBind,
    PeerSend,
    PeerUnbind,
    PeerReceive,

    MutateInstance,
    Schedule,
}

impl EffectCtor {
    #[must_use]
    pub const fn is_external_entry(self) -> bool {
        matches!(self, Self::PeerReceive)
    }
}

macro_rules! define_effect_signature {
    (
        $(
            $variant:ident {
                $($field:ident: $field_type:ty),+ $(,)?
            }
        ),+ $(,)?
    ) => {
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub enum Effect {
            $(
                $variant {
                    $($field: $field_type),+
                }
            ),+
        }

        impl Effect {
            #[must_use]
            pub const fn constructor(&self) -> EffectCtor {
                match self {
                    $(Self::$variant { .. } => EffectCtor::$variant),+
                }
            }
        }
    };
}

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum HttpMethod: u8 {
        Get = 1 => "get",
        Post = 2 => "post",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpUrlError {
    UnsupportedScheme,
    FragmentNotAllowed,
    MissingHost,
    InvalidHost,
    InvalidPort,
}

impl std::fmt::Display for HttpUrlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedScheme => formatter.write_str("HTTP URL scheme must be http or https"),
            Self::FragmentNotAllowed => formatter.write_str("HTTP URL must not contain a fragment"),
            Self::MissingHost => formatter.write_str("HTTP URL must contain a host"),
            Self::InvalidHost => formatter.write_str("HTTP URL host is invalid"),
            Self::InvalidPort => formatter.write_str("HTTP URL port is invalid"),
        }
    }
}

impl std::error::Error for HttpUrlError {}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HttpUrl {
    value: Box<str>,
    host: Box<str>,
}

impl HttpUrl {
    pub fn try_new(value: impl Into<Box<str>>) -> Result<Self, HttpUrlError> {
        let value = value.into();
        if value.contains('#') {
            return Err(HttpUrlError::FragmentNotAllowed);
        }
        let remainder = value
            .strip_prefix("https://")
            .or_else(|| value.strip_prefix("http://"))
            .ok_or(HttpUrlError::UnsupportedScheme)?;
        let authority_end = remainder.find(['/', '?']).unwrap_or(remainder.len());
        let authority = &remainder[..authority_end];
        validate_http_authority(authority)?;

        Ok(Self {
            host: authority.into(),
            value,
        })
    }

    #[must_use]
    pub const fn as_str(&self) -> &str {
        &self.value
    }

    #[must_use]
    pub const fn host(&self) -> &str {
        &self.host
    }
}

pub(crate) fn validate_http_authority(authority: &str) -> Result<(), HttpUrlError> {
    if authority.is_empty() {
        return Err(HttpUrlError::MissingHost);
    }
    if authority.contains('@') {
        return Err(HttpUrlError::InvalidHost);
    }

    let host_name = if let Some(bracketed) = authority.strip_prefix('[') {
        let closing = bracketed.find(']').ok_or(HttpUrlError::InvalidHost)?;
        let host_name = &bracketed[..closing];
        let suffix = &bracketed[closing + 1..];
        if host_name.is_empty() {
            return Err(HttpUrlError::MissingHost);
        }
        if !suffix.is_empty() {
            let port = suffix.strip_prefix(':').ok_or(HttpUrlError::InvalidHost)?;
            validate_http_port(port)?;
        }
        host_name
    } else if let Some((host_name, port)) = authority.rsplit_once(':') {
        if host_name.contains(':') {
            return Err(HttpUrlError::InvalidHost);
        }
        if host_name.is_empty() {
            return Err(HttpUrlError::MissingHost);
        }
        validate_http_port(port)?;
        host_name
    } else {
        authority
    };
    if host_name.is_empty() {
        return Err(HttpUrlError::MissingHost);
    }
    if host_name.bytes().any(|byte| {
        byte.is_ascii_control()
            || byte.is_ascii_whitespace()
            || matches!(byte, b'/' | b'\\' | b'?' | b'#' | b'[' | b']')
    }) {
        return Err(HttpUrlError::InvalidHost);
    }
    Ok(())
}

fn validate_http_port(port: &str) -> Result<(), HttpUrlError> {
    if port.is_empty()
        || !port.bytes().all(|byte| byte.is_ascii_digit())
        || port.parse::<u16>().is_err()
    {
        return Err(HttpUrlError::InvalidPort);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpHeaderError {
    InvalidName,
    InvalidValue,
    InvalidSecretResource,
}

impl std::fmt::Display for HttpHeaderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName => {
                formatter.write_str("HTTP header name is not lowercase canonical form")
            }
            Self::InvalidValue => formatter.write_str("HTTP header value contains a line break"),
            Self::InvalidSecretResource => formatter.write_str(
                "HTTP header secret resource name must be nonempty and contain no control characters",
            ),
        }
    }
}

impl std::error::Error for HttpHeaderError {}

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HttpHeaderValue {
    Plain(Box<str>),
    Secret(Box<str>),
}

impl std::fmt::Debug for HttpHeaderValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plain(value) => formatter.debug_tuple("Plain").field(value).finish(),
            Self::Secret(name) => write!(formatter, "Secret(secret:{name})"),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HttpHeader {
    name: Box<str>,
    value: HttpHeaderValue,
}

impl HttpHeader {
    pub fn try_new(
        name: impl Into<Box<str>>,
        value: impl Into<Box<str>>,
    ) -> Result<Self, HttpHeaderError> {
        let name = name.into();
        if name.is_empty() || !name.bytes().all(is_lowercase_http_token_byte) {
            return Err(HttpHeaderError::InvalidName);
        }
        let value = value.into();
        if value.contains(['\r', '\n']) {
            return Err(HttpHeaderError::InvalidValue);
        }
        Ok(Self {
            name,
            value: HttpHeaderValue::Plain(value),
        })
    }

    pub fn try_new_secret(
        name: impl Into<Box<str>>,
        resource: impl Into<Box<str>>,
    ) -> Result<Self, HttpHeaderError> {
        let name = name.into();
        if name.is_empty() || !name.bytes().all(is_lowercase_http_token_byte) {
            return Err(HttpHeaderError::InvalidName);
        }
        let resource = resource.into();
        if resource.is_empty() || resource.chars().any(char::is_control) {
            return Err(HttpHeaderError::InvalidSecretResource);
        }
        Ok(Self {
            name,
            value: HttpHeaderValue::Secret(resource),
        })
    }

    #[must_use]
    pub const fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn value(&self) -> &HttpHeaderValue {
        &self.value
    }
}

fn is_lowercase_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_lowercase()
        || byte.is_ascii_digit()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpRequestSpecError {
    GetBodyNotEmpty,
}

impl std::fmt::Display for HttpRequestSpecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GetBodyNotEmpty => formatter.write_str("HTTP GET request body must be empty"),
        }
    }
}

impl std::error::Error for HttpRequestSpecError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpRequestSpec {
    method: HttpMethod,
    url: HttpUrl,
    headers: Box<[HttpHeader]>,
    body: Box<[u8]>,
}

impl HttpRequestSpec {
    pub fn try_new(
        method: HttpMethod,
        url: HttpUrl,
        headers: impl Into<Box<[HttpHeader]>>,
        body: impl Into<Box<[u8]>>,
    ) -> Result<Self, HttpRequestSpecError> {
        let body = body.into();
        if method == HttpMethod::Get && !body.is_empty() {
            return Err(HttpRequestSpecError::GetBodyNotEmpty);
        }
        Ok(Self {
            method,
            url,
            headers: headers.into(),
            body,
        })
    }

    #[must_use]
    pub const fn method(&self) -> HttpMethod {
        self.method
    }

    #[must_use]
    pub const fn url(&self) -> &HttpUrl {
        &self.url
    }

    #[must_use]
    pub const fn headers(&self) -> &[HttpHeader] {
        &self.headers
    }

    #[must_use]
    pub const fn body(&self) -> &[u8] {
        &self.body
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ByteRange {
    start: u64,
    length: u64,
}

impl ByteRange {
    #[must_use]
    pub const fn new(start: u64, length: u64) -> Self {
        Self { start, length }
    }

    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    #[must_use]
    pub const fn length(self) -> u64 {
        self.length
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileReadSpec {
    path: NormalizedPath,
    range: Option<ByteRange>,
}

impl FileReadSpec {
    #[must_use]
    pub const fn new(path: NormalizedPath, range: Option<ByteRange>) -> Self {
        Self { path, range }
    }

    #[must_use]
    pub const fn path(&self) -> &NormalizedPath {
        &self.path
    }

    #[must_use]
    pub const fn range(&self) -> Option<ByteRange> {
        self.range
    }
}

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum FileWriteMode: u8 {
        Create = 1,
        Replace = 2,
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileWriteSpec {
    path: NormalizedPath,
    body: Box<[u8]>,
    mode: FileWriteMode,
}

impl FileWriteSpec {
    #[must_use]
    pub fn new(path: NormalizedPath, body: impl Into<Box<[u8]>>, mode: FileWriteMode) -> Self {
        Self {
            path,
            body: body.into(),
            mode,
        }
    }

    #[must_use]
    pub const fn path(&self) -> &NormalizedPath {
        &self.path
    }

    #[must_use]
    pub const fn body(&self) -> &[u8] {
        &self.body
    }

    #[must_use]
    pub const fn mode(&self) -> FileWriteMode {
        self.mode
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScheduleCorrelation(u64);

impl ScheduleCorrelation {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScheduleSpec {
    after: NonZeroMillis,
    correlation: ScheduleCorrelation,
}

impl ScheduleSpec {
    #[must_use]
    pub const fn new(after: NonZeroMillis, correlation: ScheduleCorrelation) -> Self {
        Self { after, correlation }
    }

    #[must_use]
    pub const fn after(self) -> NonZeroMillis {
        self.after
    }

    #[must_use]
    pub const fn correlation(self) -> ScheduleCorrelation {
        self.correlation
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProgramName(Box<str>);

impl ProgramName {
    #[must_use]
    pub fn from_normalized(name: impl Into<Box<str>>) -> Self {
        Self(name.into())
    }

    #[must_use]
    pub const fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProcessSpec {
    program: ProgramName,
    arguments: Box<[Box<str>]>,
    stdin: Box<[u8]>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NotificationChannel(Box<str>);

impl NotificationChannel {
    #[must_use]
    pub fn from_normalized(channel: impl Into<Box<str>>) -> Self {
        Self(channel.into())
    }

    #[must_use]
    pub const fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NotificationSpec {
    channel: NotificationChannel,
    title: Box<str>,
    body: Box<str>,
}

impl NotificationSpec {
    #[must_use]
    pub fn new(
        channel: NotificationChannel,
        title: impl Into<Box<str>>,
        body: impl Into<Box<str>>,
    ) -> Self {
        Self {
            channel,
            title: title.into(),
            body: body.into(),
        }
    }

    #[must_use]
    pub const fn channel(&self) -> &NotificationChannel {
        &self.channel
    }

    #[must_use]
    pub const fn title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub const fn body(&self) -> &str {
        &self.body
    }
}

impl ProcessSpec {
    #[must_use]
    pub fn new(
        program: ProgramName,
        arguments: impl IntoIterator<Item = impl Into<Box<str>>>,
        stdin: impl Into<Box<[u8]>>,
    ) -> Self {
        Self {
            program,
            arguments: arguments.into_iter().map(Into::into).collect(),
            stdin: stdin.into(),
        }
    }

    #[must_use]
    pub const fn program(&self) -> &ProgramName {
        &self.program
    }

    pub fn arguments(&self) -> impl ExactSizeIterator<Item = &str> {
        self.arguments.iter().map(AsRef::as_ref)
    }

    #[must_use]
    pub const fn stdin(&self) -> &[u8] {
        &self.stdin
    }
}

define_effect_signature! {
    Http {
        grant: Granted<HttpFetch>,
        ticket: Option<ApprovalTicket>,
        spec: HttpRequestSpec,
    },
    FileRead {
        grant: Granted<FsRead>,
        ticket: Option<ApprovalTicket>,
        spec: FileReadSpec,
    },
    FileWrite {
        grant: Granted<FsWrite>,
        ticket: Option<ApprovalTicket>,
        spec: FileWriteSpec,
    },
    Spawn {
        grant: Granted<ProcessSpawn>,
        ticket: Option<ApprovalTicket>,
        spec: ProcessSpec,
    },
    Notify {
        grant: Granted<UserNotify>,
        ticket: Option<ApprovalTicket>,
        spec: NotificationSpec,
    },
    AgentInvoke {
        grant: Granted<AgentHarness>,
        ticket: Option<ApprovalTicket>,
        invoke: AgentInvokeSpec,
    },
    RequestApproval {
        grant: Granted<ApprovalRequest>,
        spec: ApprovalSpec,
    },
    MutateInstance {
        spec: crate::InstanceMutationSpec,
    },
    Schedule {
        spec: ScheduleSpec,
    },
}

impl Effect {
    #[must_use]
    pub const fn http(
        grant: Granted<HttpFetch>,
        ticket: Option<ApprovalTicket>,
        spec: HttpRequestSpec,
    ) -> Self {
        Self::Http {
            grant,
            ticket,
            spec,
        }
    }

    #[must_use]
    pub const fn file_read(
        grant: Granted<FsRead>,
        ticket: Option<ApprovalTicket>,
        spec: FileReadSpec,
    ) -> Self {
        Self::FileRead {
            grant,
            ticket,
            spec,
        }
    }

    #[must_use]
    pub const fn file_write(
        grant: Granted<FsWrite>,
        ticket: Option<ApprovalTicket>,
        spec: FileWriteSpec,
    ) -> Self {
        Self::FileWrite {
            grant,
            ticket,
            spec,
        }
    }

    #[must_use]
    pub const fn spawn(
        grant: Granted<ProcessSpawn>,
        ticket: Option<ApprovalTicket>,
        spec: ProcessSpec,
    ) -> Self {
        Self::Spawn {
            grant,
            ticket,
            spec,
        }
    }

    #[must_use]
    pub const fn notify(
        grant: Granted<UserNotify>,
        ticket: Option<ApprovalTicket>,
        spec: NotificationSpec,
    ) -> Self {
        Self::Notify {
            grant,
            ticket,
            spec,
        }
    }

    #[must_use]
    pub const fn agent_invoke(
        grant: Granted<AgentHarness>,
        ticket: Option<ApprovalTicket>,
        invoke: AgentInvokeSpec,
    ) -> Self {
        Self::AgentInvoke {
            grant,
            ticket,
            invoke,
        }
    }

    #[must_use]
    pub const fn request_approval(grant: Granted<ApprovalRequest>, spec: ApprovalSpec) -> Self {
        Self::RequestApproval { grant, spec }
    }

    #[must_use]
    pub const fn approval_ticket(&self) -> Option<&ApprovalTicket> {
        match self {
            Self::Http { ticket, .. }
            | Self::FileRead { ticket, .. }
            | Self::FileWrite { ticket, .. }
            | Self::Spawn { ticket, .. }
            | Self::Notify { ticket, .. }
            | Self::AgentInvoke { ticket, .. } => ticket.as_ref(),
            Self::RequestApproval { .. } | Self::MutateInstance { .. } | Self::Schedule { .. } => {
                None
            }
        }
    }

    #[must_use]
    pub const fn mutate_instance(spec: crate::InstanceMutationSpec) -> Self {
        Self::MutateInstance { spec }
    }

    #[must_use]
    pub const fn schedule(spec: ScheduleSpec) -> Self {
        Self::Schedule { spec }
    }
}

#[must_use]
pub fn required_capability(effect: &Effect) -> Option<Capability> {
    match effect {
        Effect::Http { .. } => Some(Capability::HttpFetch),
        Effect::FileRead { .. } => Some(Capability::FsRead),
        Effect::FileWrite { .. } => Some(Capability::FsWrite),
        Effect::Spawn { .. } => Some(Capability::ProcessSpawn),
        Effect::Notify { .. } => Some(Capability::UserNotify),
        Effect::AgentInvoke { .. } => Some(Capability::AgentHarness),
        Effect::RequestApproval { .. } => Some(Capability::ApprovalRequest),
        Effect::MutateInstance { .. } | Effect::Schedule { .. } => None,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Effects(Box<[Effect]>);

impl Effects {
    #[must_use]
    pub fn empty() -> Self {
        Self(Box::new([]))
    }

    #[must_use]
    pub fn singleton(effect: Effect) -> Self {
        Self(Box::new([effect]))
    }

    #[must_use]
    pub fn concat(self, other: Self) -> Self {
        let mut terms = self.0.into_vec();
        terms.extend(other.0);
        Self(terms.into_boxed_slice())
    }

    #[must_use]
    pub const fn as_slice(&self) -> &[Effect] {
        &self.0
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Effect> {
        self.0.iter()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<Effect> for Effects {
    fn from_iter<T: IntoIterator<Item = Effect>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for Effects {
    type Item = Effect;
    type IntoIter = std::vec::IntoIter<Effect>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_vec().into_iter()
    }
}

#[must_use]
pub fn caps(effects: &Effects) -> CapabilitySet {
    effects.iter().fold(CapabilitySet::empty(), |set, effect| {
        required_capability(effect).map_or(set.clone(), |capability| {
            set.join(&CapabilitySet::singleton(capability))
        })
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActorEffect<P = ()> {
    Emit {
        port: PortId,
        payload: P,
        key: Option<crate::InstanceKey>,
        result: crate::EnvelopeResult,
    },
    Suppress {
        reason: SuppressionReason,
    },
    DeadLetter {
        subject: P,
        reason: DeclaredReason,
    },
    Reject {
        subject: P,
        cause: crate::ProcessingCause,
    },
    External(Effect),
    Peer(crate::PeerEffect),
}

impl<P> ActorEffect<P> {
    #[must_use]
    pub const fn emit(port: PortId, payload: P) -> Self {
        Self::Emit {
            port,
            payload,
            key: None,
            result: crate::EnvelopeResult::Ok,
        }
    }

    pub const fn emit_result(port: PortId, payload: P, result: crate::EnvelopeResult) -> Self {
        Self::Emit {
            port,
            payload,
            key: None,
            result,
        }
    }

    #[must_use]
    pub const fn suppress(reason: SuppressionReason) -> Self {
        Self::Suppress { reason }
    }

    #[must_use]
    pub const fn dead_letter(subject: P, reason: DeclaredReason) -> Self {
        Self::DeadLetter { subject, reason }
    }

    #[must_use]
    pub const fn external(effect: Effect) -> Self {
        Self::External(effect)
    }

    #[must_use]
    pub const fn as_external(&self) -> Option<&Effect> {
        match self {
            Self::Emit { .. }
            | Self::Suppress { .. }
            | Self::DeadLetter { .. }
            | Self::Reject { .. }
            | Self::Peer(_) => None,
            Self::External(effect) => Some(effect),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorEffects<P = ()>(Box<[ActorEffect<P>]>);

impl<P> Default for ActorEffects<P> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<P> ActorEffects<P> {
    #[must_use]
    pub fn empty() -> Self {
        Self(Box::new([]))
    }

    #[must_use]
    pub fn singleton(effect: ActorEffect<P>) -> Self {
        Self(Box::new([effect]))
    }

    #[must_use]
    pub fn emit(port: PortId, payload: P) -> Self {
        Self::singleton(ActorEffect::emit(port, payload))
    }

    #[must_use]
    pub fn reject(subject: P, cause: crate::ProcessingCause) -> Self {
        Self::singleton(ActorEffect::Reject { subject, cause })
    }

    #[must_use]
    pub fn external(effect: Effect) -> Self {
        Self::singleton(ActorEffect::external(effect))
    }

    #[must_use]
    pub fn concat(self, other: Self) -> Self {
        let mut terms = self.0.into_vec();
        terms.extend(other.0);
        Self(terms.into_boxed_slice())
    }

    #[must_use]
    pub const fn as_slice(&self) -> &[ActorEffect<P>] {
        &self.0
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &ActorEffect<P>> {
        self.0.iter()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<P> FromIterator<ActorEffect<P>> for ActorEffects<P> {
    fn from_iter<T: IntoIterator<Item = ActorEffect<P>>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<P> IntoIterator for ActorEffects<P> {
    type Item = ActorEffect<P>;
    type IntoIter = std::vec::IntoIter<ActorEffect<P>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_vec().into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(name: &str) -> PortId {
        PortId::try_new(name).expect("test port is canonical")
    }

    #[test]
    fn http_url_accepts_only_http_absolute_hosts_without_fragments() {
        let plain = HttpUrl::try_new("http://example.test/path").expect("http absolute URL");
        assert_eq!(plain.host(), "example.test");
        assert_eq!(plain.as_str(), "http://example.test/path");

        let port = HttpUrl::try_new("https://example.test:8443?q=1")
            .expect("https absolute URL with port");
        assert_eq!(port.host(), "example.test:8443");

        assert_eq!(
            HttpUrl::try_new("ftp://example.test"),
            Err(HttpUrlError::UnsupportedScheme)
        );
        assert_eq!(
            HttpUrl::try_new("/relative/path"),
            Err(HttpUrlError::UnsupportedScheme)
        );
        assert_eq!(
            HttpUrl::try_new("https:///missing-host"),
            Err(HttpUrlError::MissingHost)
        );
        assert_eq!(
            HttpUrl::try_new("https://example.test/path#fragment"),
            Err(HttpUrlError::FragmentNotAllowed)
        );
    }

    #[test]
    fn http_request_rejects_get_with_a_body() {
        let url = HttpUrl::try_new("https://example.test").expect("absolute URL");
        assert_eq!(
            HttpRequestSpec::try_new(HttpMethod::Get, url, [], b"body".to_vec()),
            Err(HttpRequestSpecError::GetBodyNotEmpty)
        );
    }

    #[test]
    fn http_header_value_is_closed_and_secret_debug_contains_only_its_name() {
        let plain = HttpHeader::try_new("accept", "application/json").expect("plain header");
        assert_eq!(
            plain.value(),
            &HttpHeaderValue::Plain("application/json".into())
        );

        let secret = HttpHeader::try_new_secret("authorization", "slack-bot")
            .expect("secret reference header");
        assert_eq!(secret.value(), &HttpHeaderValue::Secret("slack-bot".into()));
        let debug = format!("{secret:?}");
        assert_eq!(
            debug,
            r#"HttpHeader { name: "authorization", value: Secret(secret:slack-bot) }"#
        );
    }

    #[test]
    fn secret_header_rejects_empty_or_control_bearing_resource_names() {
        for resource in ["", "line\nbreak", "carriage\rreturn", "control\u{0007}"] {
            assert_eq!(
                HttpHeader::try_new_secret("authorization", resource),
                Err(HttpHeaderError::InvalidSecretResource)
            );
        }
        assert_eq!(
            HttpHeader::try_new_secret("Authorization", "slack-bot"),
            Err(HttpHeaderError::InvalidName)
        );
    }

    #[test]
    fn actor_emit_has_an_explicit_payload_and_cannot_be_read_as_external() {
        let effect: ActorEffect<u8> = ActorEffect::emit(port("pulse"), 7);
        assert_eq!(effect.as_external(), None);
        assert!(matches!(
            effect,
            ActorEffect::Emit { ref port, payload: 7, .. } if port.as_str() == "pulse"
        ));
    }

    #[test]
    fn actor_effects_preserve_emit_external_emit_order_and_caps_fold_external_only() {
        use crate::{FsWriteGrant, GrantIssuer, PathScope, PathScopes};

        let path = NormalizedPath::new("/scope/output").unwrap();
        let grant = FsWriteGrant::fs_write(PathScopes::new([PathScope::new(path.clone())]));
        let write = Effect::file_write(
            GrantIssuer::new().issue(&grant),
            None,
            FileWriteSpec::new(path, [], FileWriteMode::Replace),
        );
        let effects: ActorEffects<u8> = [
            ActorEffect::emit(port("first"), 1),
            ActorEffect::external(write),
            ActorEffect::emit(port("last"), 2),
        ]
        .into_iter()
        .collect();

        assert!(matches!(
            &effects.as_slice()[0],
            ActorEffect::Emit { port, payload: 1, .. } if port.as_str() == "first"
        ));
        assert!(matches!(
            &effects.as_slice()[1],
            ActorEffect::External(Effect::FileWrite { .. })
        ));
        assert!(matches!(
            &effects.as_slice()[2],
            ActorEffect::Emit { port, payload: 2, .. } if port.as_str() == "last"
        ));

        let external: Effects = effects
            .into_iter()
            .filter_map(|effect| match effect {
                ActorEffect::Emit { .. }
                | ActorEffect::Suppress { .. }
                | ActorEffect::DeadLetter { .. }
                | ActorEffect::Reject { .. }
                | ActorEffect::Peer(_) => None,
                ActorEffect::External(effect) => Some(effect),
            })
            .collect();
        assert_eq!(
            caps(&external),
            CapabilitySet::singleton(Capability::FsWrite)
        );
    }
}

#[cfg(test)]
mod boarded_internal_effects {
    use super::*;
    use crate::{InstanceAuthority, InstanceIntent, InstanceKey, InstanceMutationSpec};
    use circular_plan::{InstanceScalar, Name, ScopeId, ScopeRole, admit_template};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Seal {}

    fn schedule_spec() -> ScheduleSpec {
        ScheduleSpec::new(
            NonZeroMillis::new(120_000).expect("positive milliseconds"),
            ScheduleCorrelation::new(3),
        )
    }

    fn mutation_spec() -> InstanceMutationSpec {
        let plan = {
            let mut table = circular_plan::ScopeRoleTable::new();
            table.declare(
                ScopeId::from_segments(vec![circular_plan::ScopeSeg::Child(
                    Name::from_normalized("cell"),
                )])
                .unwrap(),
                ScopeRole::Template,
            );
            table
        };
        let authority = InstanceAuthority::<Seal>::granted(
            &admit_template(&plan, &ScopeId::root(), &Name::from_normalized("cell")).unwrap(),
        );
        InstanceMutationSpec::requested(
            &authority,
            InstanceIntent::Instantiate {
                key: InstanceKey::Scalar(InstanceScalar::normalized_text("s1")),
            },
        )
    }

    #[test]
    fn internal_effects_require_no_outward_capability() {
        assert_eq!(
            required_capability(&Effect::schedule(schedule_spec())),
            None
        );
        assert_eq!(
            required_capability(&Effect::mutate_instance(mutation_spec())),
            None
        );
    }

    #[test]
    fn the_two_arms_carry_their_own_constructor_tags() {
        assert_eq!(
            Effect::schedule(schedule_spec()).constructor(),
            EffectCtor::Schedule
        );
        assert_eq!(
            Effect::mutate_instance(mutation_spec()).constructor(),
            EffectCtor::MutateInstance
        );
    }

    #[test]
    fn neither_arm_carries_an_approval_ticket() {
        assert!(
            Effect::schedule(schedule_spec())
                .approval_ticket()
                .is_none()
        );
        assert!(
            Effect::mutate_instance(mutation_spec())
                .approval_ticket()
                .is_none()
        );
    }

    #[test]
    fn a_schedule_says_how_long_not_when() {
        let spec = schedule_spec();
        assert_eq!(spec.after().get().get(), 120_000);
        assert_eq!(spec, ScheduleSpec::new(spec.after(), spec.correlation()));
    }

    #[test]
    fn the_correlation_is_the_actors_own_value() {
        let armed = ScheduleSpec::new(
            NonZeroMillis::new(1).expect("positive"),
            ScheduleCorrelation::new(7),
        );
        let rearmed = ScheduleSpec::new(
            NonZeroMillis::new(1).expect("positive"),
            ScheduleCorrelation::new(8),
        );
        assert_ne!(armed, rearmed);
        assert_eq!(armed.correlation().get(), 7);
    }
}
