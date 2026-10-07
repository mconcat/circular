
use std::num::NonZeroU64;

use circular_core::{Ceilings, CodecError, ObjectValue, Value, encode};

use crate::declaration_payload::result::Rejected;
use crate::wire_value::{
    PayloadRejection, arm, decode_arm, exhausted, object, object_fields, optional, take, text_of,
    unit_arm, unsigned_of,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub name: String,
    pub args: Value,
    pub page: Option<PageRequest>,
    pub since: Option<crate::replay_payload::LogCut>,
    pub upto: Option<crate::replay_payload::LogCut>,
    pub lens: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageRequest {
    pub limit: NonZeroU64,
    pub cursor: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Terminal {
    More(Value),
    Complete,
    Diagnostic(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryPage {
    pub anchor: Value,
    pub items: Vec<Value>,
    pub terminal: Terminal,
    pub reached: Option<Value>,
    pub cut: Option<Value>,
    pub folded_from: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum QueryResult {
    Page(QueryPage),
    Rejected(Rejected),
}

impl Terminal {
    fn to_value(&self) -> Value {
        match self {
            Self::More(cursor) => arm(1, cursor.clone()),
            Self::Complete => unit_arm(2),
            Self::Diagnostic(code) => arm(3, Value::Int(i64::from(*code))),
        }
    }

    pub fn from_value(value: Value) -> Result<Self, PayloadRejection> {
        let (tag, mut arguments) = decode_arm(value, "terminal")?;
        match tag {
            1 if arguments.len() == 1 => Ok(Self::More(arguments.pop().expect("one argument"))),
            2 if arguments.is_empty() => Ok(Self::Complete),
            3 if arguments.len() == 1 => {
                let code = unsigned_of(arguments.pop().expect("one argument"), "terminal")?;
                Ok(Self::Diagnostic(u32::try_from(code).map_err(|_| {
                    PayloadRejection::BeyondWidth { key: "terminal" }
                })?))
            }
            1..=3 => Err(PayloadRejection::WrongCarrier { key: "terminal" }),
            other => Err(PayloadRejection::UnknownArm { tag: other }),
        }
    }
}

impl Query {
    pub fn to_value(&self) -> Result<Value, PayloadRejection> {
        let mut entries = vec![
            ("args".to_owned(), self.args.clone()),
            ("name".to_owned(), Value::String(self.name.clone())),
        ];
        if let Some(since) = &self.since {
            entries.push(("since".to_owned(), since.to_value()?));
        }
        if let Some(upto) = &self.upto {
            entries.push(("upto".to_owned(), upto.to_value()?));
        }
        if let Some(lens) = self.lens {
            entries.push(("lens".to_owned(), Value::Int(i64::from(lens))));
        }
        if let Some(page) = &self.page {
            let limit = i64::try_from(page.limit.get())
                .map_err(|_| PayloadRejection::BeyondWidth { key: "limit" })?;
            let mut page_entries = vec![("limit".to_owned(), Value::Int(limit))];
            if let Some(cursor) = &page.cursor {
                page_entries.push(("cursor".to_owned(), cursor.clone()));
            }
            entries.push((
                "page".to_owned(),
                Value::Object(
                    ObjectValue::try_from_entries(page_entries).expect("page request keys differ"),
                ),
            ));
        }
        Ok(Value::Object(
            ObjectValue::try_from_entries(entries).expect("query keys differ"),
        ))
    }

    pub fn encode(&self, ceilings: Ceilings) -> Result<Vec<u8>, PayloadRejection> {
        encode(&self.to_value()?, ceilings).map_err(PayloadRejection::Codec)
    }
}

pub fn decode_query(bytes: &[u8], ceilings: Ceilings) -> Result<Query, PayloadRejection> {
    let mut fields = object(bytes, ceilings)?;
    let args = take(&mut fields, "args")?;
    let name = text_of(take(&mut fields, "name")?, "name")?;

    let page = match optional(&mut fields, "page") {
        Some(page) => {
            let mut page_fields = object_fields(page, "page")?;
            let cursor = optional(&mut page_fields, "cursor");
            let limit = unsigned_of(take(&mut page_fields, "limit")?, "limit")?;
            exhausted(page_fields)?;
            let limit =
                NonZeroU64::new(limit).ok_or(PayloadRejection::WrongCarrier { key: "limit" })?;
            Some(PageRequest { limit, cursor })
        }
        None => None,
    };
    let since = take_cut(&mut fields, "since", ceilings)?;
    let upto = take_cut(&mut fields, "upto", ceilings)?;
    let lens = optional(&mut fields, "lens")
        .map(|lens| {
            u32::try_from(unsigned_of(lens, "lens")?)
                .map_err(|_| PayloadRejection::WrongCarrier { key: "lens" })
        })
        .transpose()?;
    exhausted(fields)?;

    Ok(Query {
        name,
        args,
        page,
        since,
        upto,
        lens,
    })
}

impl QueryResult {
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Page(page) => {
                let mut entries = vec![
                    ("anchor".to_owned(), page.anchor.clone()),
                    ("items".to_owned(), Value::Array(page.items.clone())),
                    ("terminal".to_owned(), page.terminal.to_value()),
                ];
                for (name, cut) in [("cut", &page.cut), ("folded_from", &page.folded_from)] {
                    if let Some(cut) = cut {
                        entries.push((name.to_owned(), cut.clone()));
                    }
                }
                if let Some(reached) = &page.reached {
                    entries.push(("reached".to_owned(), reached.clone()));
                }
                arm(
                    1,
                    Value::Object(
                        ObjectValue::try_from_entries(entries).expect("distinct page keys"),
                    ),
                )
            }
            Self::Rejected(rejected) => arm(2, rejected.to_value()),
        }
    }

    pub fn encode(&self, ceilings: Ceilings) -> Result<Vec<u8>, CodecError> {
        encode(&self.to_value(), ceilings)
    }

    pub fn decode(bytes: &[u8], ceilings: Ceilings) -> Result<Self, PayloadRejection> {
        let value = circular_core::decode(bytes, ceilings).map_err(PayloadRejection::Codec)?;
        let (tag, mut arguments) = decode_arm(value, "result")?;
        match tag {
            1 => {
                if arguments.len() != 1 {
                    return Err(PayloadRejection::WrongCarrier { key: "result" });
                }
                let payload = arguments.pop().expect("one argument");
                let mut fields = object_fields(payload, "page")?;
                let anchor = take(&mut fields, "anchor")?;
                let items = match take(&mut fields, "items")? {
                    Value::Array(items) => items,
                    _ => return Err(PayloadRejection::WrongCarrier { key: "items" }),
                };
                let terminal = Terminal::from_value(take(&mut fields, "terminal")?)?;
                let reached = optional(&mut fields, "reached");
                if let Some(value) = &reached {
                    validate_reached(value.clone(), ceilings)?;
                }
                let cut = take_cut_value(&mut fields, "cut", ceilings)?;
                let folded_from = take_cut_value(&mut fields, "folded_from", ceilings)?;
                exhausted(fields)?;
                Ok(Self::Page(QueryPage {
                    cut,
                    folded_from,
                    reached,
                    anchor,
                    items,
                    terminal,
                }))
            }
            2 => {
                if arguments.len() != 1 {
                    return Err(PayloadRejection::WrongCarrier { key: "result" });
                }
                Rejected::from_value(arguments.pop().expect("one argument")).map(Self::Rejected)
            }
            other => Err(PayloadRejection::UnknownArm { tag: other }),
        }
    }
}

fn take_cut_value(
    fields: &mut Vec<(String, Value)>,
    key: &'static str,
    ceilings: Ceilings,
) -> Result<Option<Value>, PayloadRejection> {
    let value = optional(fields, key);
    if let Some(value) = &value {
        crate::replay_payload::LogCut::from_value(value.clone(), ceilings)?;
    }
    Ok(value)
}

fn take_cut(
    fields: &mut Vec<(String, Value)>,
    key: &'static str,
    ceilings: Ceilings,
) -> Result<Option<crate::replay_payload::LogCut>, PayloadRejection> {
    optional(fields, key)
        .map(|value| crate::replay_payload::LogCut::from_value(value, ceilings))
        .transpose()
}

fn validate_reached(value: Value, ceilings: Ceilings) -> Result<(), PayloadRejection> {
    let mut fields = object_fields(value, "reached")?;
    match take(&mut fields, "revision_epoch")? {
        Value::UInt(revision) if revision > 0 => (),
        _ => {
            return Err(PayloadRejection::WrongCarrier {
                key: "revision_epoch",
            });
        }
    }
    crate::replay_payload::LogCut::from_value(take(&mut fields, "cut")?, ceilings)?;
    Ok(exhausted(fields)?)
}

#[cfg(test)]
mod query_tests {
    use super::*;
    use circular_core::{ObjectValue, decode, encode};

    const CEILINGS: Ceilings = Ceilings::for_boundary(circular_core::Boundary::Wire);

    fn body(entries: Vec<(String, Value)>) -> Vec<u8> {
        encode(
            &Value::Object(ObjectValue::try_from_entries(entries).expect("the keys differ")),
            CEILINGS,
        )
        .expect("encodes")
    }

    #[test]
    fn a_query_without_paging_omits_the_key() {
        let bytes = body(vec![
            ("args".to_owned(), Value::Null),
            ("name".to_owned(), Value::String("actors".to_owned())),
        ]);
        let decoded = decode_query(&bytes, CEILINGS).expect("decodes");
        assert_eq!(decoded.name, "actors");
        assert!(decoded.page.is_none(), "absence is structure");
    }

    #[test]
    fn a_page_request_carries_its_limit() {
        let page = Value::Object(
            ObjectValue::try_from_entries([("limit".to_owned(), Value::Int(50))]).expect("one key"),
        );
        let bytes = body(vec![
            ("args".to_owned(), Value::Null),
            ("name".to_owned(), Value::String("n".to_owned())),
            ("page".to_owned(), page),
        ]);
        let decoded = decode_query(&bytes, CEILINGS).expect("decodes");
        let request = decoded.page.expect("requested a page");
        assert_eq!(request.limit.get(), 50);
        assert!(request.cursor.is_none(), "the first page has no cursor");
    }

    #[test]
    fn a_query_encoder_crosses_to_the_existing_decoder() {
        let query = Query {
            since: None,
            upto: None,
            lens: None,
            name: "actors".to_owned(),
            args: Value::Object(
                ObjectValue::try_from_entries([("scope".to_owned(), Value::Null)])
                    .expect("one key"),
            ),
            page: Some(PageRequest {
                limit: NonZeroU64::new(50).expect("positive"),
                cursor: Some(Value::bytes(vec![7, 8])),
            }),
        };

        assert_eq!(
            decode_query(&query.encode(CEILINGS).expect("encodes"), CEILINGS),
            Ok(query)
        );
    }

    #[test]
    fn a_zero_limit_cannot_form_a_page_request() {
        assert!(NonZeroU64::new(0).is_none());
    }

    #[test]
    fn a_zero_limit_is_refused() {
        let page = Value::Object(
            ObjectValue::try_from_entries([("limit".to_owned(), Value::Int(0))]).expect("one key"),
        );
        let bytes = body(vec![
            ("args".to_owned(), Value::Null),
            ("name".to_owned(), Value::String("n".to_owned())),
            ("page".to_owned(), page),
        ]);
        assert_eq!(
            decode_query(&bytes, CEILINGS),
            Err(PayloadRejection::WrongCarrier { key: "limit" })
        );
    }

    #[test]
    fn a_more_terminal_leads_with_its_tag_and_carries_the_cursor() {
        let more = QueryResult::Page(QueryPage {
            cut: None,
            folded_from: None,
            reached: None,
            anchor: Value::Null,
            items: vec![Value::Int(1)],
            terminal: Terminal::More(Value::bytes(vec![9])),
        });
        let Value::Array(parts) = more.to_value() else {
            panic!("it is a sequence");
        };
        let Value::Object(page) = &parts[1] else {
            panic!("it is an object");
        };
        assert_eq!(
            page.clone().into_map().remove("terminal"),
            Some(Value::Array(vec![Value::Int(1), Value::Bytes(vec![9])]))
        );
    }

    #[test]
    fn a_query_result_encoder_crosses_to_the_new_decoder() {
        let page = QueryResult::Page(QueryPage {
            cut: None,
            folded_from: None,
            reached: None,
            anchor: Value::String("standing".to_owned()),
            items: vec![Value::Int(1), Value::Int(2)],
            terminal: Terminal::More(Value::bytes(vec![9])),
        });
        assert_eq!(
            QueryResult::decode(&page.encode(CEILINGS).expect("encodes"), CEILINGS),
            Ok(page)
        );

        let rejected = QueryResult::Rejected(Rejected {
            code: 7,
            message: "no such query".to_owned(),
            hint: Some("register it".to_owned()),
            at: Some(Value::Null),
        });
        assert_eq!(
            QueryResult::decode(&rejected.encode(CEILINGS).expect("encodes"), CEILINGS),
            Ok(rejected)
        );
    }

    #[test]
    fn terminal_tags_are_closed_and_complete_has_no_argument() {
        for tag in [0, 4] {
            assert_eq!(
                Terminal::from_value(Value::Int(tag)),
                Err(PayloadRejection::UnknownArm { tag })
            );
        }
        assert_eq!(
            Terminal::from_value(Value::Array(vec![Value::Int(2), Value::Null])),
            Err(PayloadRejection::WrongCarrier { key: "terminal" })
        );
    }

    #[test]
    fn a_diagnostic_code_cannot_exceed_u32() {
        let code = i64::from(u32::MAX) + 1;
        assert_eq!(
            Terminal::from_value(Value::Array(vec![Value::Int(3), Value::Int(code)])),
            Err(PayloadRejection::BeyondWidth { key: "terminal" })
        );
    }

    #[test]
    fn rejected_optional_fields_are_absent_by_structure() {
        let without_optionals = Value::Object(
            ObjectValue::try_from_entries([
                ("code".to_owned(), Value::Int(7)),
                ("message".to_owned(), Value::String("no".to_owned())),
            ])
            .expect("two keys"),
        );
        let decoded = Rejected::from_value(without_optionals).expect("refusal envelope");
        assert_eq!(decoded.hint, None);
        assert_eq!(decoded.at, None);

        let null_hint = Value::Object(
            ObjectValue::try_from_entries([
                ("code".to_owned(), Value::Int(7)),
                ("hint".to_owned(), Value::Null),
                ("message".to_owned(), Value::String("no".to_owned())),
            ])
            .expect("three keys"),
        );
        assert_eq!(
            Rejected::from_value(null_hint),
            Err(PayloadRejection::WrongCarrier { key: "hint" })
        );

        let null_at = Value::Object(
            ObjectValue::try_from_entries([
                ("at".to_owned(), Value::Null),
                ("code".to_owned(), Value::Int(7)),
                ("message".to_owned(), Value::String("no".to_owned())),
            ])
            .expect("three keys"),
        );
        assert_eq!(
            Rejected::from_value(null_at)
                .expect("Null is a value too")
                .at,
            Some(Value::Null)
        );
    }

    #[test]
    fn a_query_page_rejects_unpublished_keys() {
        let page = Value::Object(
            ObjectValue::try_from_entries([
                ("anchor".to_owned(), Value::Null),
                ("items".to_owned(), Value::Array(Vec::new())),
                ("terminal".to_owned(), Value::Int(2)),
                ("unpublished".to_owned(), Value::Null),
            ])
            .expect("four keys"),
        );
        let bytes = encode(&arm(1, page), CEILINGS).expect("encodes");
        assert_eq!(
            QueryResult::decode(&bytes, CEILINGS),
            Err(PayloadRejection::UnknownKey("unpublished".to_owned()))
        );
    }

    #[test]
    fn a_rejected_query_round_trips() {
        let rejected = QueryResult::Rejected(Rejected {
            code: 7,
            message: "no such query".to_owned(),
            hint: None,
            at: None,
        });
        let bytes = rejected.encode(CEILINGS).expect("encodes");
        let Value::Array(parts) = decode(&bytes, CEILINGS).expect("it is a value") else {
            panic!("it is a sequence");
        };
        assert_eq!(parts[0], Value::Int(2), "tag of the refusal");
    }
}

#[cfg(test)]
mod invalid_reached_tests {
    use super::*;
    #[test]
    fn reached_rejects_null_unknown_fields_and_untyped_revision() {
        for reached in [
            Value::Null,
            Value::object([
                ("cut", Value::Array(vec![])),
                ("revision_epoch", Value::Int(1)),
            ])
            .unwrap(),
            Value::object([
                ("cut", Value::Array(vec![])),
                ("revision_epoch", Value::UInt(0)),
            ])
            .unwrap(),
            Value::object([
                ("cut", Value::Array(vec![])),
                ("revision_epoch", Value::UInt(1)),
                ("extra", Value::Null),
            ])
            .unwrap(),
        ] {
            let page = QueryResult::Page(QueryPage {
                cut: None,
                folded_from: None,
                anchor: Value::Null,
                items: vec![],
                terminal: Terminal::Complete,
                reached: Some(reached),
            });
            assert!(
                QueryResult::decode(
                    &page
                        .encode(Ceilings::for_boundary(circular_core::Boundary::Wire))
                        .unwrap(),
                    Ceilings::for_boundary(circular_core::Boundary::Wire)
                )
                .is_err()
            );
        }
    }
}

