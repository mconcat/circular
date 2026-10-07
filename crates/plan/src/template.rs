//! A template retains the existing canonical DeclarationCommand value bytes.
use crate::Name;
use circular_core::{Boundary, Ceilings, Value, decode, encode};
use circular_protocol::declaration_payload::{PayloadRejection, upsert_template_from_value};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Template {
    name: Name,
    commands: Box<[u8]>,
}

impl Template {
    pub fn try_new(name: Name, commands: Vec<Value>) -> Result<Self, PayloadRejection> {
        let value = Value::object([
            ("name", Value::string(name.as_str())),
            ("commands", Value::Array(commands.clone())),
        ])
        .expect("two distinct template fields");
        upsert_template_from_value(value)?;
        let commands = encode(
            &Value::Array(commands),
            Ceilings::for_boundary(Boundary::Journal),
        )
        .map_err(PayloadRejection::Codec)?
        .into_boxed_slice();
        Ok(Self { name, commands })
    }

    pub fn name(&self) -> &Name {
        &self.name
    }
    pub fn commands_bytes(&self) -> &[u8] {
        &self.commands
    }
    pub fn commands(&self) -> Vec<Value> {
        let Value::Array(commands) =
            decode(&self.commands, Ceilings::for_boundary(Boundary::Journal))
                .expect("sealed canonical declaration array")
        else {
            unreachable!("sealed array")
        };
        commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_value_retains_canonical_command_bytes_without_source_or_runtime_state() {
        const COMMANDS: &[u8] = &[
            7, 0, 0, 0, 1, 8, 0, 0, 0, 3, 0, 0, 0, 5, 97, 99, 116, 111, 114, 7, 0, 0, 0, 2, 3, 0,
            0, 0, 0, 0, 0, 0, 3, 8, 0, 0, 0, 2, 0, 0, 0, 5, 108, 111, 99, 97, 108, 5, 0, 0, 0, 3,
            116, 97, 112, 0, 0, 0, 5, 115, 99, 111, 112, 101, 7, 0, 0, 0, 0, 0, 0, 0, 11, 100, 101,
            99, 108, 97, 114, 97, 116, 105, 111, 110, 8, 0, 0, 0, 2, 0, 0, 0, 6, 100, 111, 109, 97,
            105, 110, 8, 0, 0, 0, 2, 0, 0, 0, 10, 97, 99, 116, 111, 114, 95, 116, 121, 112, 101, 5,
            0, 0, 0, 3, 116, 97, 112, 0, 0, 0, 6, 99, 111, 110, 102, 105, 103, 1, 0, 0, 0, 5, 102,
            108, 97, 103, 115, 8, 0, 0, 0, 3, 0, 0, 0, 6, 98, 121, 112, 97, 115, 115, 2, 0, 0, 0,
            0, 4, 109, 117, 116, 101, 2, 0, 0, 0, 0, 5, 112, 97, 117, 115, 101, 2, 0, 0, 0, 0, 4,
            107, 105, 110, 100, 5, 0, 0, 0, 11, 85, 112, 115, 101, 114, 116, 65, 99, 116, 111, 114,
        ];
        let Value::Array(input) =
            decode(COMMANDS, Ceilings::for_boundary(Boundary::Journal)).unwrap()
        else {
            panic!("literal command array")
        };
        let template = Template::try_new(Name::from_normalized("worker"), input).unwrap();
        assert_eq!(template.name().as_str(), "worker");
        assert_eq!(template.commands_bytes(), COMMANDS);
        assert_eq!(template.commands().len(), 1);
        let Value::Object(command) = template.commands().remove(0) else {
            panic!("declaration object")
        };
        assert_eq!(command.get("kind"), Some(&Value::string("UpsertActor")));
        assert_eq!(template, template.clone());
    }
}
