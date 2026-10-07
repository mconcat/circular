
use crate::{
    BaseShape, GroundShape, Name, ProductPayload, ProductValue, Shape, effect_failure_detail,
};
use circular_runtime::{
    AgentPayload, AgentToolCall, AgentToolCallId, AgentToolResult, Capability,
    ConcreteExternalEffectTag, ConcreteOutcomePayload, Divergence, EffectFailure, InterpreterFault,
    ProcessResult, ToolName,
};

fn ground(shape: Shape) -> GroundShape {
    GroundShape::try_new(shape).expect("tool wire shapes contain no variables")
}

const fn failure_tag(failure: &EffectFailure) -> &'static str {
    match failure {
        EffectFailure::ParameterDenied { .. } => "parameter_denied",
        EffectFailure::Diverged(_) => "diverged",
        EffectFailure::EndpointGone => "endpoint_gone",
        EffectFailure::ApprovalRequired => "approval_required",
        EffectFailure::TransportTerminal
        | EffectFailure::RetryExhausted { .. }
        | EffectFailure::TransportUnreached
        | EffectFailure::RemoteDeferred => "transport_terminal",
        EffectFailure::InterpreterFault(_)
        | EffectFailure::Peer(_) => "interpreter_fault",
    }
}

fn parse_failure_tag(text: &str, detail: Option<&ProductValue>) -> Option<EffectFailure> {
    Some(match text {
        "parameter_denied" => {
            let ProductValue::UInt(position) = detail? else {
                return None;
            };
            let index = usize::try_from(position.checked_sub(1)?).ok()?;
            EffectFailure::ParameterDenied {
                capability: *Capability::ALL.get(index)?,
            }
        }
        "diverged" => {
            let ProductValue::UInt(tag) = detail? else {
                return None;
            };
            EffectFailure::Diverged(Divergence::from_tag(u8::try_from(*tag).ok()?)?)
        }
        "endpoint_gone" => EffectFailure::EndpointGone,
        "approval_required" => EffectFailure::ApprovalRequired,
        "transport_terminal" => EffectFailure::TransportTerminal,
        "interpreter_fault" => EffectFailure::InterpreterFault(InterpreterFault::Other),
        _ => return None,
    })
}

fn lenient_bytes(value: &ProductValue) -> Option<Vec<u8>> {
    match value {
        ProductValue::Bytes(bytes) => Some(bytes.clone()),
        ProductValue::String(text) => Some(text.as_bytes().to_vec()),
        _ => None,
    }
}

#[must_use]
pub fn tool_call_payload(call: &AgentToolCall) -> ProductPayload {
    ProductPayload::new(
        ground(Shape::Object {
            fields: crate::FieldMap::try_new(vec![
                (Name::from_static("id"), Shape::Base(BaseShape::Bytes)),
                (Name::from_static("tool"), Shape::Base(BaseShape::String)),
                (
                    Name::from_static("arguments"),
                    Shape::Base(BaseShape::Bytes),
                ),
            ])
            .expect("tool-call fields are unique"),
            open: false,
        }),
        ProductValue::object([
            ("id", ProductValue::Bytes(call.id().as_bytes().to_vec())),
            (
                "tool",
                ProductValue::String(call.tool().as_str().to_owned()),
            ),
            (
                "arguments",
                ProductValue::Bytes(call.arguments().as_bytes().to_vec()),
            ),
        ])
        .expect("tool-call fields are unique"),
    )
}

#[must_use]
pub fn decode_tool_call(payload: &ProductPayload) -> Option<AgentToolCall> {
    let object = payload.value().as_object()?;
    let id = AgentToolCallId::try_from_bytes(lenient_bytes(object.get("id")?)?).ok()?;
    let tool = ToolName::try_from_normalized(object.get("tool")?.as_str()?.to_owned()).ok()?;
    let arguments = AgentPayload::new(lenient_bytes(object.get("arguments")?)?);
    Some(AgentToolCall::new(id, tool, arguments))
}

#[must_use]
pub fn tool_result_payload(result: &AgentToolResult) -> ProductPayload {
    let (ok, value) = match result.result() {
        Ok(ConcreteOutcomePayload::FileBytes(bytes)) => (true, ProductValue::Bytes(bytes.to_vec())),
        Ok(ConcreteOutcomePayload::WrittenLength(length)) => (true, ProductValue::UInt(*length)),
        Ok(ConcreteOutcomePayload::ProcessResult(process)) => (
            true,
            ProductValue::object([
                ("exit", ProductValue::Int(i64::from(process.exit_code()))),
                ("stdout", ProductValue::Bytes(process.stdout().to_vec())),
                ("stderr", ProductValue::Bytes(process.stderr().to_vec())),
            ])
            .expect("process result fields are unique"),
        ),
        Err(failure) => (false, ProductValue::String(failure_tag(failure).to_owned())),
    };
    let mut fields = vec![
        (Name::from_static("call"), Shape::Base(BaseShape::Bytes)),
        (Name::from_static("effect"), Shape::Base(BaseShape::String)),
        (Name::from_static("ok"), Shape::Base(BaseShape::Bool)),
        (Name::from_static("value"), Shape::Any),
    ];
    let mut values = vec![
        (
            "call",
            ProductValue::Bytes(result.call().as_bytes().to_vec()),
        ),
        (
            "effect",
            ProductValue::String(result.effect().as_str().to_owned()),
        ),
        ("ok", ProductValue::Bool(ok)),
        ("value", value),
    ];
    if let Err(failure @ (EffectFailure::ParameterDenied { .. } | EffectFailure::Diverged(_))) =
        result.result()
    {
        fields.push((Name::from_static("detail"), Shape::Base(BaseShape::UInt)));
        values.push(("detail", effect_failure_detail(failure)));
    }
    ProductPayload::new(
        ground(Shape::Object {
            fields: crate::FieldMap::try_new(fields).expect("tool-result fields are unique"),
            open: false,
        }),
        ProductValue::object(values).expect("tool-result fields are unique"),
    )
}

#[must_use]
pub fn decode_tool_result(payload: &ProductPayload) -> Option<AgentToolResult> {
    let object = payload.value().as_object()?;
    let call = AgentToolCallId::try_from_bytes(lenient_bytes(object.get("call")?)?).ok()?;
    let effect = ConcreteExternalEffectTag::from_str(object.get("effect")?.as_str()?)?;
    if object.get("ok")?.as_bool()? {
        let value = object.get("value")?;
        let outcome = match effect {
            ConcreteExternalEffectTag::FileRead => {
                ConcreteOutcomePayload::FileBytes(value.as_bytes()?.to_vec().into())
            }
            ConcreteExternalEffectTag::FileWrite => {
                let ProductValue::UInt(length) = value else {
                    return None;
                };
                ConcreteOutcomePayload::WrittenLength(*length)
            }
            ConcreteExternalEffectTag::Spawn => {
                let process = value.as_object()?;
                let ProductValue::Int(exit) = process.get("exit")? else {
                    return None;
                };
                ConcreteOutcomePayload::ProcessResult(ProcessResult::direct(
                    i32::try_from(*exit).ok()?,
                    process.get("stdout")?.as_bytes()?.to_vec(),
                    process.get("stderr")?.as_bytes()?.to_vec(),
                ))
            }
        };
        AgentToolResult::succeeded(call, effect, outcome).ok()
    } else {
        let failure = parse_failure_tag(object.get("value")?.as_str()?, object.get("detail"))?;
        Some(AgentToolResult::failed(call, effect, failure))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &[u8]) -> AgentToolCall {
        AgentToolCall::new(
            AgentToolCallId::try_from_bytes(id.to_vec()).expect("nonempty call id"),
            ToolName::try_from_normalized("write-note".to_owned()).expect("nonempty tool"),
            AgentPayload::new(b"arguments".to_vec()),
        )
    }

    #[test]
    fn a_tool_call_round_trips_through_the_wire_payload() {
        let sent = call(b"call-1");
        let decoded =
            decode_tool_call(&tool_call_payload(&sent)).expect("encoded call decodes back");
        assert_eq!(decoded, sent);
    }

    #[test]
    fn every_success_payload_kind_round_trips() {
        let results = [
            AgentToolResult::succeeded(
                call(b"c1").id().clone(),
                ConcreteExternalEffectTag::FileRead,
                ConcreteOutcomePayload::FileBytes(b"contents".to_vec().into()),
            )
            .expect("matching pair"),
            AgentToolResult::succeeded(
                call(b"c2").id().clone(),
                ConcreteExternalEffectTag::FileWrite,
                ConcreteOutcomePayload::WrittenLength(16),
            )
            .expect("matching pair"),
            AgentToolResult::succeeded(
                call(b"c3").id().clone(),
                ConcreteExternalEffectTag::Spawn,
                ConcreteOutcomePayload::ProcessResult(ProcessResult::direct(
                    3,
                    b"out".to_vec(),
                    b"err".to_vec(),
                )),
            )
            .expect("matching pair"),
        ];
        for sent in results {
            let decoded = decode_tool_result(&tool_result_payload(&sent))
                .expect("encoded result decodes back");
            assert_eq!(decoded, sent);
        }
    }

    #[test]
    fn named_failure_classes_round_trip_and_the_rest_collapse_explicitly() {
        for failure in [
            EffectFailure::EndpointGone,
            EffectFailure::ApprovalRequired,
            EffectFailure::TransportTerminal,
            EffectFailure::Diverged(Divergence::MissingRecord),
            EffectFailure::Diverged(Divergence::EffectMismatch),
        ]
        .into_iter()
        .chain(
            Capability::ALL
                .into_iter()
                .map(|capability| EffectFailure::ParameterDenied { capability }),
        ) {
            let sent = AgentToolResult::failed(
                call(b"c4").id().clone(),
                ConcreteExternalEffectTag::FileWrite,
                failure.clone(),
            );
            let decoded = decode_tool_result(&tool_result_payload(&sent))
                .expect("encoded failure decodes back");
            assert_eq!(decoded.result(), &Err(failure));
        }

        let collapsed = AgentToolResult::failed(
            call(b"c5").id().clone(),
            ConcreteExternalEffectTag::FileWrite,
            EffectFailure::InterpreterFault(InterpreterFault::NotFound),
        );
        let decoded = decode_tool_result(&tool_result_payload(&collapsed))
            .expect("collapsed failure still decodes");
        assert_eq!(
            decoded.result(),
            &Err(EffectFailure::InterpreterFault(InterpreterFault::Other)),
            "interpreter fault details still collapse to one explicit fault"
        );
    }

    fn failure_input(tag: &str, detail: Option<ProductValue>) -> ProductPayload {
        let mut fields = vec![
            ("call", ProductValue::Bytes(b"recorded-call".to_vec())),
            ("effect", ProductValue::String("file_write".to_owned())),
            ("ok", ProductValue::Bool(false)),
            ("value", ProductValue::String(tag.to_owned())),
        ];
        if let Some(detail) = detail {
            fields.push(("detail", detail));
        }
        ProductPayload::new(
            ground(Shape::Any),
            ProductValue::object(fields).expect("distinct wire fields"),
        )
    }

    #[test]
    fn failures_with_missing_or_invalid_detail_are_refused() {
        for tag in ["parameter_denied", "diverged"] {
            for detail in [
                None,
                Some(ProductValue::Null),
                Some(ProductValue::UInt(0)),
                Some(ProductValue::UInt(256)),
                Some(ProductValue::UInt(u64::MAX)),
                Some(ProductValue::Int(1)),
                Some(ProductValue::String("1".to_owned())),
            ] {
                let input = failure_input(tag, detail);
                assert_eq!(decode_tool_result(&input), None, "{:?}", input.value());
            }
        }
        assert_eq!(
            decode_tool_result(&failure_input("diverged", Some(ProductValue::UInt(3)))),
            None
        );
    }

    #[test]
    fn recorded_interpreter_fault_is_not_reinterpreted_as_a_new_failure() {
        let result = decode_tool_result(&failure_input("interpreter_fault", None))
            .expect("recorded interpreter fault still decodes");
        assert_eq!(
            result.result(),
            &Err(EffectFailure::InterpreterFault(InterpreterFault::Other))
        );
        assert_eq!(
            decode_tool_result(&failure_input("unknown_failure", None)),
            None
        );
    }
}
