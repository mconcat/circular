
#[cfg(test)]
use crate::Shape;
use crate::actor_support::error_payload;
use crate::config::{ConfigRejection, Slot, Spelled};
use crate::{ActorType, ERROR_PORT_NAME, Flow, GroundShape, ProductPayload, ProductValue};
#[cfg(test)]
use circular_core::Tick;
use circular_core::{Fields, PortId};
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorTypes, Capability, ConfigChangeOutcome,
    EditableActor, Effect, EffectFailure, EffectOutcome, EmittingActor, EmittingActorFactory,
    FoldedConfig, HttpFetchAuthorityBearer, HttpHeader, HttpHeaderError, HttpHeaderValue,
    HttpMethod, HttpRequestSpec, HttpResponse, HttpUrl, HttpUrlError, OutcomePayload,
    ProcessingCause,
};
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::sync::LazyLock;

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("registered request port names are canonical")
}

#[cfg(test)]
fn payload_shape(shape: Shape) -> GroundShape {
    GroundShape::try_new(shape).expect("request product shapes contain no variables")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestPayloadError {
    PostBodyOutOfDomain,
}

impl fmt::Display for RequestPayloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PostBodyOutOfDomain => {
                formatter.write_str("request POST body must be bytes or string")
            }
        }
    }
}

impl Error for RequestPayloadError {}

pub(crate) const METHOD: Slot<Spelled<HttpMethod>> =
    Slot::new("method", Spelled::new(&HttpMethod::ALL, HttpMethod::as_str));

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestConfig {
    method: HttpMethod,
    url: HttpUrl,
    headers: Box<[HttpHeader]>,
}

static RESPONSE_SHAPE: LazyLock<GroundShape> = LazyLock::new(|| {
    let outlet = crate::get(ActorType::Request)
        .ports()
        .fixed()
        .outlets()
        .iter()
        .find(|outlet| outlet.id().as_str() == "response")
        .expect("response outlet of the request registration");
    let Flow::Stream(shape) = outlet.ty() else {
        unreachable!("response is a Stream")
    };
    GroundShape::try_new(shape.clone()).expect("the registered response is a ground shape")
});

fn response_payload(response: &HttpResponse) -> ProductPayload {
    let retry_after_seconds = response
        .retry_after_seconds()
        .map_or(ProductValue::Null, ProductValue::UInt);
    ProductPayload::new(
        RESPONSE_SHAPE.clone(),
        ProductValue::object([
            ("status", ProductValue::UInt(u64::from(response.status()))),
            ("body", ProductValue::Bytes(response.body().to_vec())),
            ("truncated", ProductValue::Bool(response.truncated())),
            ("retry_after_seconds", retry_after_seconds),
        ])
        .expect("request response field names are distinct"),
    )
}

pub struct RequestActor<V, I> {
    config: RequestConfig,
    pending: usize,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> RequestActor<V, I> {
    #[must_use]
    pub const fn pending(&self) -> usize {
        self.pending
    }

    fn failure_text(&self, failure: &EffectFailure) -> String {
        let code = failure.kind_tag();
        if !matches!(
            failure,
            EffectFailure::ParameterDenied {
                capability: Capability::HttpFetch
            }
        ) {
            return format!("request failed: {code}");
        }
        let authority = self.config.url.host();
        let every_port = circular_runtime::authority_host(authority);
        let mut text = format!(
            "request failed: {code}; hint: at least one of these does not hold: [http] hosts in config.toml lists \"{authority}\" or \"{every_port}:*\""
        );
        if self.config.url.as_str().starts_with("http://") {
            text.push_str(
                "; a plain http:// URL names a loopback host (use https:// for any other host)",
            );
        }
        if self
            .config
            .headers
            .iter()
            .any(|header| matches!(header.value(), HttpHeaderValue::Secret(_)))
        {
            text.push_str(
                "; each secret header's name is in the secret vault and its value is UTF-8 text with no ASCII control character other than tab",
            );
        }
        text.push_str(
            "; the response body contains no value from the secret vault (checked after the request was sent)",
        );
        text
    }
}

impl<V: Clone, I: Clone + Ord> EditableActor for RequestActor<V, I> {
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, _config: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::ReplaceIncarnation
    }
}

impl<T> EmittingActor<T, ProductPayload> for RequestActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Grants: HttpFetchAuthorityBearer,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        if input.inlet() != &port("event") {
            return ActorEffects::reject(
                error_payload("unknown request inlet"),
                ProcessingCause::InputOutOfDomain,
            );
        }
        let Some(grant) = context.grants().http_fetch_authority() else {
            return ActorEffects::emit(port(ERROR_PORT_NAME), error_payload("HttpFetch denied"));
        };
        let body = match self.config.method {
            HttpMethod::Get => Vec::new(),
            HttpMethod::Post => match input.payload::<T>().value() {
                ProductValue::Bytes(bytes) => bytes.clone(),
                ProductValue::String(text) => text.as_bytes().to_vec(),
                _ => {
                    return ActorEffects::reject(
                        error_payload(RequestPayloadError::PostBodyOutOfDomain.to_string()),
                        ProcessingCause::InputOutOfDomain,
                    );
                }
            },
        };
        let spec = HttpRequestSpec::try_new(
            self.config.method,
            self.config.url.clone(),
            self.config.headers.clone(),
            body,
        )
        .expect("GET has an empty body and POST takes any body size");
        self.pending += 1;
        ActorEffects::external(Effect::http(grant, None, spec))
    }

    fn on_outcome(
        &mut self,
        outcome: &EffectOutcome<T::EffectId>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        debug_assert!(
            self.pending > 0,
            "engine EffectNotPending guard must reject an uncorrelated request outcome"
        );
        self.pending = self.pending.saturating_sub(1);
        let (outlet, payload) = match outcome.result() {
            Ok(OutcomePayload::HttpResponse(response)) => ("response", response_payload(response)),
            Ok(other) => (
                ERROR_PORT_NAME,
                error_payload(format!(
                    "request received mismatched outcome kind {}",
                    other.kind_tag()
                )),
            ),
            Err(failure) => (ERROR_PORT_NAME, error_payload(self.failure_text(failure))),
        };
        ActorEffects::emit(port(outlet), payload)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestFactoryError {
    InvalidConfig,
    Config(ConfigRejection),
    InvalidUrlType,
    InvalidUrl(HttpUrlError),
    InvalidHeadersType,
    InvalidHeaderEntry {
        index: usize,
    },
    UnexpectedHeaderKey {
        index: usize,
        key: Box<str>,
    },
    MissingHeaderName {
        index: usize,
    },
    InvalidHeaderNameType {
        index: usize,
    },
    MissingHeaderValueOrSecret {
        index: usize,
    },
    ConflictingHeaderValueAndSecret {
        index: usize,
    },
    InvalidHeaderValueType {
        index: usize,
    },
    InvalidHeaderSecretType {
        index: usize,
    },
    InvalidHeader {
        index: usize,
        source: HttpHeaderError,
    },
}

impl fmt::Display for RequestFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("request config must be an object"),
            Self::Config(rejection) => rejection.fmt(formatter),
            Self::InvalidUrlType => formatter.write_str("request url must be a string"),
            Self::InvalidUrl(source) => write!(formatter, "request url is invalid: {source}"),
            Self::InvalidHeadersType => formatter.write_str("request headers must be an array"),
            Self::InvalidHeaderEntry { index } => {
                write!(formatter, "request header {index} must be an object")
            }
            Self::UnexpectedHeaderKey { index, key } => write!(
                formatter,
                "request header {index} contains unexpected key {key:?}"
            ),
            Self::MissingHeaderName { index } => {
                write!(formatter, "request header {index} is missing name")
            }
            Self::InvalidHeaderNameType { index } => {
                write!(formatter, "request header {index} name must be a string")
            }
            Self::MissingHeaderValueOrSecret { index } => write!(
                formatter,
                "request header {index} must contain exactly one of value or secret"
            ),
            Self::ConflictingHeaderValueAndSecret { index } => write!(
                formatter,
                "request header {index} cannot contain both value and secret"
            ),
            Self::InvalidHeaderSecretType { index } => {
                write!(formatter, "request header {index} secret must be a string")
            }
            Self::InvalidHeaderValueType { index } => {
                write!(formatter, "request header {index} value must be a string")
            }
            Self::InvalidHeader { index, source } => {
                write!(formatter, "request header {index} is invalid: {source}")
            }
        }
    }
}

impl Error for RequestFactoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidUrl(source) => Some(source),
            Self::InvalidHeader { source, .. } => Some(source),
            Self::Config(rejection) => Some(rejection),
            Self::InvalidConfig
            | Self::InvalidUrlType
            | Self::InvalidHeadersType
            | Self::InvalidHeaderEntry { .. }
            | Self::UnexpectedHeaderKey { .. }
            | Self::MissingHeaderName { .. }
            | Self::InvalidHeaderNameType { .. }
            | Self::MissingHeaderValueOrSecret { .. }
            | Self::ConflictingHeaderValueAndSecret { .. }
            | Self::InvalidHeaderValueType { .. }
            | Self::InvalidHeaderSecretType { .. } => None,
        }
    }
}

impl From<crate::config::ConfigRejection> for RequestFactoryError {
    fn from(rejection: crate::config::ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

fn parse_headers(value: &ProductValue) -> Result<Box<[HttpHeader]>, RequestFactoryError> {
    let values = value
        .as_array()
        .ok_or(RequestFactoryError::InvalidHeadersType)?;
    let mut headers = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let mut entry =
            Fields::open(value).map_err(|_| RequestFactoryError::InvalidHeaderEntry { index })?;
        let name = entry.take("name");
        let header_value = entry.take("value");
        let secret = entry.take("secret");
        entry
            .finish()
            .map_err(|unknown| RequestFactoryError::UnexpectedHeaderKey {
                index,
                key: unknown.key.into(),
            })?;
        let name = name
            .ok_or(RequestFactoryError::MissingHeaderName { index })?
            .as_str()
            .ok_or(RequestFactoryError::InvalidHeaderNameType { index })?;
        let header = match (header_value, secret) {
            (Some(_), Some(_)) => {
                return Err(RequestFactoryError::ConflictingHeaderValueAndSecret { index });
            }
            (None, None) => {
                return Err(RequestFactoryError::MissingHeaderValueOrSecret { index });
            }
            (Some(value), None) => {
                let value = value
                    .as_str()
                    .ok_or(RequestFactoryError::InvalidHeaderValueType { index })?;
                HttpHeader::try_new(name, value)
            }
            (None, Some(secret)) => {
                let secret = secret
                    .as_str()
                    .ok_or(RequestFactoryError::InvalidHeaderSecretType { index })?;
                HttpHeader::try_new_secret(name, secret)
            }
        }
        .map_err(|source| RequestFactoryError::InvalidHeader { index, source })?;
        headers.push(header);
    }
    Ok(headers.into_boxed_slice())
}

pub(crate) fn judge(
    config: &FoldedConfig,
    _inlets: &crate::inlet_shapes::ResolvedInletShapes,
) -> Result<(), RequestFactoryError> {
    parse_config(config).map(drop)
}

fn parse_config(config: &FoldedConfig) -> Result<RequestConfig, RequestFactoryError> {
    crate::retry_config::declared(config.value())
        .map_err(|_| RequestFactoryError::InvalidConfig)?;
    let value = config
        .for_type(ActorType::Request)
        .map_err(|_| RequestFactoryError::InvalidConfig)?;
    let schema = crate::registration(ActorType::Request).spec().config();
    let mut fields = schema.open(value)?;
    let method = schema.read(&mut fields, &METHOD)?;
    let url = schema.raw(&mut fields, "url")?;
    let headers = schema.raw(&mut fields, "headers")?;
    let url = url.as_str().ok_or(RequestFactoryError::InvalidUrlType)?;
    let url = HttpUrl::try_new(url).map_err(RequestFactoryError::InvalidUrl)?;
    let headers = parse_headers(headers)?;
    Ok(RequestConfig {
        method,
        url,
        headers,
    })
}

pub struct RequestFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for RequestFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Grants: HttpFetchAuthorityBearer,
{
    const TYPE: circular_core::ActorType = circular_core::ActorType::Request;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = false;
    type Grants = T::Grants;
    type Types = T;
    type Instance = RequestActor<T::StateVersion, T::EffectId>;
    type Error = RequestFactoryError;

    fn create(
        config: &FoldedConfig,
        _grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        Ok(RequestActor {
            config: parse_config(config)?,
            pending: 0,
            marker: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_plan::{
        ActorId, Config, Generation, GenerationVector, Incarnation, Name as PlanName, NamedActorId,
        ScopeId,
    };
    use circular_runtime::{
        ActorEffect, Capability, EffectFailure, GrantIssuer, Granted, HttpFetch, HttpFetchGrant,
        HttpHeaderValue, HttpHosts,
    };

    use circular_testkit::types::TestRun;

    struct TestGrants(Option<Granted<HttpFetch>>);

    impl HttpFetchAuthorityBearer for TestGrants {
        fn http_fetch_authority(&self) -> Option<Granted<HttpFetch>> {
            self.0
        }
    }

    type TestTypes = circular_testkit::types::TestTypes<ProductPayload, TestGrants>;

    struct UnreadEvent;
    struct UnreadTypes;
    impl ActorTypes for UnreadTypes {
        type Stream = TestRun;
        type Event = UnreadEvent;
        type Payload = ProductPayload;
        type EffectId = u64;
        type StateVersion = u16;
        type Observation = ();
        type Grants = TestGrants;

        fn payload(_event: &Self::Event) -> &Self::Payload {
            panic!("GET must not read its event payload")
        }
    }

    fn grants() -> TestGrants {
        let grant = HttpFetchGrant::http_fetch(HttpHosts::exact(["example.test"]));
        TestGrants(Some(GrantIssuer::new().issue(&grant)))
    }

    fn folded(value: ProductValue) -> FoldedConfig {
        FoldedConfig::minted(ActorType::Request, value)
    }

    fn config(
        method: ProductValue,
        url: ProductValue,
        headers: Option<ProductValue>,
    ) -> ProductValue {
        let mut entries = vec![("method", method), ("url", url)];
        if let Some(headers) = headers {
            entries.push(("headers", headers));
        }
        ProductValue::object(entries).expect("request config fixture keys are unique")
    }

    fn valid_config(method: &str) -> FoldedConfig {
        folded(config(
            ProductValue::String(method.to_owned()),
            ProductValue::String("http://example.test/items".to_owned()),
            Some(ProductValue::Array(vec![
                ProductValue::object([
                    ("name", ProductValue::String("x-fixture".to_owned())),
                    ("value", ProductValue::String("present".to_owned())),
                ])
                .expect("request header fixture keys are unique"),
            ])),
        ))
    }

    fn actor(method: &str, grants: &TestGrants) -> RequestActor<u16, u64> {
        <RequestFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(
            &valid_config(method),
            grants,
        )
        .expect("valid request config stands")
    }

    fn payload(value: ProductValue) -> ProductPayload {
        ProductPayload::new(payload_shape(Shape::Any), value)
    }

    fn drive<Ret>(
        grants: &TestGrants,
        f: impl FnOnce(&ActorContext<'_, TestRun, TestGrants>) -> Ret,
    ) -> Ret {
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("request"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .expect("root generation");
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor: ActorId = named.as_actor_id();
        let plan_config = Config::default();
        let context = ActorContext::new(&actor, &incarnation, &plan_config, grants);
        f(&context)
    }

    fn event(
        actor: &mut RequestActor<u16, u64>,
        context: &ActorContext<'_, TestRun, TestGrants>,
        value: ProductValue,
    ) -> ActorEffects<ProductPayload> {
        <RequestActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
            actor,
            &ActorInput::new(port("event"), payload(value)),
            context,
        )
    }

    fn settle(
        actor: &mut RequestActor<u16, u64>,
        context: &ActorContext<'_, TestRun, TestGrants>,
        correlation: u64,
        result: Result<OutcomePayload, EffectFailure>,
    ) -> ActorEffects<ProductPayload> {
        <RequestActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_outcome(
            actor,
            &EffectOutcome::new(correlation, result),
            context,
        )
    }

    fn is_error(effects: &ActorEffects<ProductPayload>) -> bool {
        matches!(
            effects.as_slice(),
            [ActorEffect::Emit { port, .. }] if port.as_str() == ERROR_PORT_NAME
        )
    }

    #[test]
    fn factory_exhaustively_rejects_invalid_method_url_headers_and_extra_keys() {
        let header = |fields: Vec<(&str, ProductValue)>| {
            ProductValue::Array(vec![
                ProductValue::object(fields).expect("header fixture keys are unique"),
            ])
        };
        let string = |value: &str| ProductValue::String(value.to_owned());
        let cases = vec![
            ("non-object", ProductValue::Null),
            (
                "extra root key",
                ProductValue::object([
                    ("method", string("get")),
                    ("url", string("http://example.test")),
                    ("retry", ProductValue::Bool(true)),
                ])
                .expect("fixture keys are unique"),
            ),
            (
                "missing method",
                ProductValue::object([("url", string("http://example.test"))])
                    .expect("fixture keys are unique"),
            ),
            (
                "missing url",
                ProductValue::object([("method", string("get"))]).expect("fixture keys are unique"),
            ),
            (
                "url type",
                config(string("get"), ProductValue::UInt(1), None),
            ),
            (
                "url constructor",
                config(string("get"), string("ftp://example.test"), None),
            ),
            (
                "headers type",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(ProductValue::Null),
                ),
            ),
            (
                "header entry type",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(ProductValue::Array(vec![ProductValue::Null])),
                ),
            ),
            (
                "header extra key",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(header(vec![
                        ("name", string("accept")),
                        ("value", string("text/plain")),
                        ("other", ProductValue::Bool(true)),
                    ])),
                ),
            ),
            (
                "missing header name",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(header(vec![("value", string("text/plain"))])),
                ),
            ),
            (
                "header name type",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(header(vec![
                        ("name", ProductValue::UInt(1)),
                        ("value", string("text/plain")),
                    ])),
                ),
            ),
            (
                "missing header value and secret",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(header(vec![("name", string("accept"))])),
                ),
            ),
            (
                "both header value and secret",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(header(vec![
                        ("name", string("authorization")),
                        ("value", string("plain")),
                        ("secret", string("slack-bot")),
                    ])),
                ),
            ),
            (
                "header value type",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(header(vec![
                        ("name", string("accept")),
                        ("value", ProductValue::UInt(1)),
                    ])),
                ),
            ),
            (
                "header secret type",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(header(vec![
                        ("name", string("authorization")),
                        ("secret", ProductValue::UInt(1)),
                    ])),
                ),
            ),
            (
                "header name constructor",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(header(vec![
                        ("name", string("Accept")),
                        ("value", string("text/plain")),
                    ])),
                ),
            ),
            (
                "header value constructor",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(header(vec![
                        ("name", string("accept")),
                        ("value", string("bad\nvalue")),
                    ])),
                ),
            ),
            (
                "header secret constructor",
                config(
                    string("get"),
                    string("http://example.test"),
                    Some(header(vec![
                        ("name", string("authorization")),
                        ("secret", string("bad\nname")),
                    ])),
                ),
            ),
        ];

        let grants = grants();
        for (label, value) in cases {
            let result =
                <RequestFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(
                    &folded(value),
                    &grants,
                );
            assert!(result.is_err(), "{label} must reject activation");
        }
        assert!(
            <RequestFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(
                &folded(config(
                    string("get"),
                    string("https://example.test/items"),
                    None,
                )),
                &grants,
            )
            .is_ok(),
            "headers defaults to an empty array"
        );
        assert!(
            <RequestFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(
                &valid_config("post"),
                &grants,
            )
            .is_ok(),
            "validated headers stand"
        );

        let secret = folded(config(
            string("get"),
            string("https://example.test/items"),
            Some(header(vec![
                ("name", string("authorization")),
                ("secret", string("slack-bot")),
            ])),
        ));
        let parsed = parse_config(&secret).expect("secret reference header stands");
        assert!(matches!(
            parsed.headers.as_ref(),
            [header]
                if header.name() == "authorization"
                    && header.value() == &HttpHeaderValue::Secret("slack-bot".into())
        ));
    }

    #[test]
    fn get_does_not_read_the_event_payload() {
        let grants = grants();
        drive(&grants, |context| {
            let mut actor = actor("get", &grants);
            let effects =
                <RequestActor<u16, u64> as EmittingActor<UnreadTypes, ProductPayload>>::on_event(
                    &mut actor,
                    &ActorInput::new(port("event"), UnreadEvent),
                    context,
                );
            assert!(matches!(
                effects.as_slice(),
                [ActorEffect::External(Effect::Http { spec, .. })]
                    if spec.method() == HttpMethod::Get && spec.body().is_empty()
            ));
            assert_eq!(actor.pending(), 1);
        });
    }

    #[test]
    fn missing_grant_and_terminal_failure_emit_error_with_failure_kind() {
        let denied = TestGrants(None);
        drive(&denied, |context| {
            let mut actor = actor("get", &denied);
            let effects = event(&mut actor, context, ProductValue::Null);
            assert!(is_error(&effects));
            assert_eq!(actor.pending(), 0);
        });

        let grants = grants();
        drive(&grants, |context| {
            let mut actor = actor("get", &grants);
            assert!(matches!(
                event(&mut actor, context, ProductValue::Null).as_slice(),
                [ActorEffect::External(Effect::Http { .. })]
            ));
            let failure = EffectFailure::ParameterDenied {
                capability: Capability::HttpFetch,
            };
            let tag = failure.kind_tag();
            let effects = settle(&mut actor, context, 1, Err(failure));
            let [ActorEffect::Emit { port, payload, .. }] = effects.as_slice() else {
                panic!("request failure must emit one error");
            };
            assert_eq!(port.as_str(), ERROR_PORT_NAME);
            assert!(
                payload
                    .value()
                    .as_str()
                    .is_some_and(|text| text.contains(tag))
            );
            assert_eq!(actor.pending(), 0);
        });
    }

    #[test]
    fn concurrent_successes_settle_to_closed_response_facts() {
        let grants = grants();
        drive(&grants, |context| {
            let mut actor = actor("get", &grants);
            for _ in 0..2 {
                assert!(matches!(
                    event(&mut actor, context, ProductValue::Null).as_slice(),
                    [ActorEffect::External(Effect::Http { .. })]
                ));
            }
            assert_eq!(actor.pending(), 2);

            let response = settle(
                &mut actor,
                context,
                2,
                Ok(OutcomePayload::HttpResponse(HttpResponse::new(
                    503,
                    b"later".to_vec(),
                    true,
                    Some(17),
                ))),
            );
            let [ActorEffect::Emit { port, payload, .. }] = response.as_slice() else {
                panic!("request response must emit one value");
            };
            assert_eq!(port.as_str(), "response");
            let object = payload.value().as_object().expect("response is an object");
            assert_eq!(object.len(), 4);
            assert_eq!(object.get("status"), Some(&ProductValue::UInt(503)));
            assert_eq!(
                object.get("body"),
                Some(&ProductValue::Bytes(b"later".to_vec()))
            );
            assert_eq!(object.get("truncated"), Some(&ProductValue::Bool(true)));
            assert_eq!(
                object.get("retry_after_seconds"),
                Some(&ProductValue::UInt(17))
            );
            assert_eq!(actor.pending(), 1);

            let without_retry = settle(
                &mut actor,
                context,
                1,
                Ok(OutcomePayload::HttpResponse(HttpResponse::new(
                    200,
                    Vec::new(),
                    false,
                    None,
                ))),
            );
            let [ActorEffect::Emit { payload, .. }] = without_retry.as_slice() else {
                panic!("request response without retry metadata emits one value");
            };
            assert_eq!(
                payload
                    .value()
                    .as_object()
                    .expect("response is an object")
                    .get("retry_after_seconds"),
                Some(&ProductValue::Null)
            );
            assert_eq!(actor.pending(), 0);
        });
    }
}
