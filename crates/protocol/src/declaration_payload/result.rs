
use circular_core::{Ceilings, CodecError, ObjectValue, Value, encode};

use crate::wire_value::{
    PayloadRejection, arm, exhausted, object_fields, optional, take, text_of, unit_arm, unsigned_of,
};

#[derive(Clone, Debug, PartialEq)]
pub enum CommandResult {
    Accepted(Accepted),
    Rejected(Rejected),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Accepted {
    Nothing,
    Epoch(Vec<u8>),
    Transition(Value),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rejected {
    pub code: u32,
    pub message: String,
    pub hint: Option<String>,
    pub at: Option<Value>,
}

impl Rejected {
    pub fn from_value(value: Value) -> Result<Self, PayloadRejection> {
        let mut fields = object_fields(value, "rejected")?;
        let code = unsigned_of(take(&mut fields, "code")?, "code")?;
        let code =
            u32::try_from(code).map_err(|_| PayloadRejection::BeyondWidth { key: "code" })?;
        let message = text_of(take(&mut fields, "message")?, "message")?;
        let hint = optional(&mut fields, "hint")
            .map(|value| text_of(value, "hint"))
            .transpose()?;
        let at = optional(&mut fields, "at");
        exhausted(fields)?;
        Ok(Self {
            code,
            message,
            hint,
            at,
        })
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut entries = vec![
            ("code".to_owned(), Value::Int(i64::from(self.code))),
            ("message".to_owned(), Value::String(self.message.clone())),
        ];
        if let Some(hint) = &self.hint {
            entries.push(("hint".to_owned(), Value::String(hint.clone())));
        }
        if let Some(at) = &self.at {
            entries.push(("at".to_owned(), at.clone()));
        }
        Value::Object(
            ObjectValue::try_from_entries(entries).expect("the refusal envelope keys differ"),
        )
    }
}

impl Accepted {
    fn to_value(&self) -> Value {
        match self {
            Self::Nothing => unit_arm(1),
            Self::Epoch(epoch) => arm(1, Value::bytes(epoch.clone())),
            Self::Transition(metadata) => arm(1, metadata.clone()),
        }
    }
}

impl CommandResult {
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Accepted(accepted) => accepted.to_value(),
            Self::Rejected(rejected) => arm(2, rejected.to_value()),
        }
    }

    pub fn encode(&self, ceilings: Ceilings) -> Result<Vec<u8>, CodecError> {
        encode(&self.to_value(), ceilings)
    }
}

#[cfg(test)]
mod command_result_tests {
    use super::*;
    use circular_core::decode;

    const CEILINGS: Ceilings = Ceilings::for_boundary(circular_core::Boundary::Wire);

    fn decoded(result: &CommandResult) -> Value {
        decode(&result.encode(CEILINGS).expect("encodes"), CEILINGS).expect("it is a value")
    }

    #[test]
    fn an_acceptance_with_no_fact_is_the_tag_itself() {
        assert_eq!(
            decoded(&CommandResult::Accepted(Accepted::Nothing)),
            Value::Int(1)
        );
    }

    #[test]
    fn an_acceptance_with_a_fact_leads_with_its_tag() {
        assert_eq!(
            decoded(&CommandResult::Accepted(Accepted::Epoch(vec![7, 8]))),
            Value::Array(vec![Value::Int(1), Value::Bytes(vec![7, 8])])
        );
    }

    #[test]
    fn absent_hint_and_site_are_structural() {
        let without = decoded(&CommandResult::Rejected(Rejected {
            code: 1,
            message: "m".to_owned(),
            hint: None,
            at: None,
        }));
        let with = decoded(&CommandResult::Rejected(Rejected {
            code: 1,
            message: "m".to_owned(),
            hint: Some("try again".to_owned()),
            at: None,
        }));
        assert_ne!(without, with, "absence and a value have the same bytes");

        let Value::Array(parts) = with else {
            panic!("it is a sequence");
        };
        let Value::Object(object) = &parts[1] else {
            panic!("it is an object");
        };
        let keys = object.clone().into_map().into_keys().collect::<Vec<_>>();
        assert_eq!(
            keys,
            ["code", "hint", "message"],
            "original UTF-8 key order"
        );
    }

    #[test]
    fn a_result_has_no_freedom_left() {
        let result = CommandResult::Rejected(Rejected {
            code: 42,
            message: "m".to_owned(),
            hint: Some("h".to_owned()),
            at: None,
        });
        assert_eq!(
            result.encode(CEILINGS).expect("encodes"),
            result.encode(CEILINGS).expect("encodes")
        );
    }
}
