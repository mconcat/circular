
use circular_core::{Ceilings, Value, encode};

use crate::wire_value::{PayloadRejection, exhausted, object, object_fields, take, text_of};

pub const AGENT_HARNESSES_QUERY: &str = "agent.harnesses";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetAgentHarness {
    pub name: String,
    pub program: Option<String>,
}

fn text_or_null(value: Value, key: &'static str) -> Result<Option<String>, PayloadRejection> {
    match value {
        Value::Null => Ok(None),
        value => text_of(value, key).map(Some),
    }
}

fn nullable(text: Option<&str>) -> Value {
    text.map_or(Value::Null, Value::string)
}

fn fields<const N: usize>(entries: [(&'static str, Value); N]) -> Value {
    Value::object(entries).expect("the published keys are distinct")
}

pub fn encode_set_agent_harness(
    request: &SetAgentHarness,
    ceilings: Ceilings,
) -> Result<Vec<u8>, PayloadRejection> {
    encode(
        &fields([
            ("name", Value::string(&request.name)),
            ("program", nullable(request.program.as_deref())),
        ]),
        ceilings,
    )
    .map_err(PayloadRejection::Codec)
}

pub fn decode_set_agent_harness(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<SetAgentHarness, PayloadRejection> {
    let mut fields = object(bytes, ceilings)?;
    let name = text_of(take(&mut fields, "name")?, "name")?;
    let program = text_or_null(take(&mut fields, "program")?, "program")?;
    exhausted(fields)?;
    Ok(SetAgentHarness { name, program })
}

#[must_use]
pub fn set_agent_harness_accepted_value() -> Value {
    fields([("at", Value::Null)])
}

pub fn decode_set_agent_harness_accepted(value: Value) -> Result<(), PayloadRejection> {
    let mut fields = object_fields(value, "accepted")?;
    match take(&mut fields, "at")? {
        Value::Null => {}
        _ => return Err(PayloadRejection::WrongCarrier { key: "at" }),
    }
    exhausted(fields)?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentHarnessRow {
    pub name: String,
    pub program: Option<String>,
    pub saved: Option<String>,
}

#[must_use]
pub fn agent_harness_row_value(row: &AgentHarnessRow) -> Value {
    fields([
        ("name", Value::string(&row.name)),
        ("program", nullable(row.program.as_deref())),
        ("saved", nullable(row.saved.as_deref())),
    ])
}

pub fn decode_agent_harness_row(value: Value) -> Result<AgentHarnessRow, PayloadRejection> {
    let mut fields = object_fields(value, "row")?;
    let name = text_of(take(&mut fields, "name")?, "name")?;
    let program = text_or_null(take(&mut fields, "program")?, "program")?;
    let saved = text_or_null(take(&mut fields, "saved")?, "saved")?;
    exhausted(fields)?;
    Ok(AgentHarnessRow {
        name,
        program,
        saved,
    })
}

