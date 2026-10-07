//! Fail-closed pre-ledger scrubbing for generic OTLP JSON.
//!
//! The denylist applies both to OTLP `KeyValue` attributes and to ordinary JSON
//! object fields. Sensitive attributes are removed; email addresses and
//! absolute paths embedded in otherwise useful strings are replaced. An event
//! is rejected when an attribute shape or sensitive-looking string cannot be
//! classified without guessing.

use std::fmt;

use serde_json::{Map, Value};

const REDACTED_EMAIL: &str = "<redacted:email>";
const REDACTED_PATH: &str = "<redacted:absolute-path>";

#[derive(Clone, Debug, PartialEq)]
pub struct ScrubbedEvent {
    value: Value,
    scrubbed_fields: u64,
}

impl ScrubbedEvent {
    pub fn into_parts(self) -> (Value, u64) {
        (self.value, self.scrubbed_fields)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrubError {
    InvalidAttribute,
    InvalidAttributeKey,
    InvalidDroppedAttributesCount,
    ScrubCountOverflow,
}

impl fmt::Display for ScrubError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let diagnostic = match self {
            Self::InvalidAttribute => "OTLP attribute shape is not classifiable",
            Self::InvalidAttributeKey => "OTLP attribute key is not a string",
            Self::InvalidDroppedAttributesCount => {
                "OTLP droppedAttributesCount is not an unsigned integer"
            }
            Self::ScrubCountOverflow => "scrubbed field count overflow",
        };
        formatter.write_str(diagnostic)
    }
}

impl std::error::Error for ScrubError {}

#[derive(Clone, Copy, Debug, Default)]
pub struct PreLedgerScrubber;

impl PreLedgerScrubber {
    pub fn scrub(&self, mut event: Value) -> Result<ScrubbedEvent, ScrubError> {
        let mut scrubbed_fields = 0_u64;
        scrub_value(&mut event, &[], &mut scrubbed_fields)?;
        Ok(ScrubbedEvent {
            value: event,
            scrubbed_fields,
        })
    }
}

fn scrub_value(
    value: &mut Value,
    ancestors: &[String],
    scrubbed_fields: &mut u64,
) -> Result<(), ScrubError> {
    match value {
        Value::String(text) => {
            let redacted = redact_sensitive_text(text)?;
            if redacted != *text {
                increment(scrubbed_fields, 1)?;
                *text = redacted;
            }
        }
        Value::Array(values) => {
            for value in values {
                scrub_value(value, ancestors, scrubbed_fields)?;
            }
        }
        Value::Object(fields) => scrub_object(fields, ancestors, scrubbed_fields)?,
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn scrub_object(
    fields: &mut Map<String, Value>,
    ancestors: &[String],
    scrubbed_fields: &mut u64,
) -> Result<(), ScrubError> {
    if fields
        .get("droppedAttributesCount")
        .is_some_and(|value| value.as_u64().is_none())
    {
        return Err(ScrubError::InvalidDroppedAttributesCount);
    }
    if let Some(mut attributes) = fields.remove("attributes") {
        let removed = scrub_attributes(&mut attributes, ancestors, scrubbed_fields)?;
        fields.insert("attributes".to_owned(), attributes);
        if removed != 0 && fields.contains_key("droppedAttributesCount") {
            increment_dropped_attributes(fields, removed)?;
        }
    }

    if let Some(kvlist) = fields.get_mut("kvlistValue") {
        scrub_kvlist(kvlist, ancestors, scrubbed_fields)?;
    }

    let keys = fields.keys().cloned().collect::<Vec<_>>();
    for key in keys {
        if matches!(key.as_str(), "attributes" | "kvlistValue") {
            continue;
        }
        let tokens = key_tokens(&key);
        if object_field_is_denied(&tokens, ancestors) {
            fields.remove(&key);
            increment(scrubbed_fields, 1)?;
            continue;
        }
        let mut child_ancestors = ancestors.to_vec();
        child_ancestors.extend(tokens);
        if let Some(value) = fields.get_mut(&key) {
            scrub_value(value, &child_ancestors, scrubbed_fields)?;
        }
    }
    Ok(())
}

fn scrub_attributes(
    value: &mut Value,
    ancestors: &[String],
    scrubbed_fields: &mut u64,
) -> Result<u64, ScrubError> {
    let attributes = value.as_array_mut().ok_or(ScrubError::InvalidAttribute)?;
    let mut kept = Vec::with_capacity(attributes.len());
    let mut removed = 0_u64;
    for mut attribute in attributes.drain(..) {
        let fields = attribute
            .as_object_mut()
            .ok_or(ScrubError::InvalidAttribute)?;
        if fields.len() != 2 || !fields.contains_key("value") {
            return Err(ScrubError::InvalidAttribute);
        }
        let key = fields
            .get("key")
            .and_then(Value::as_str)
            .ok_or(ScrubError::InvalidAttributeKey)?;
        let tokens = key_tokens(key);
        if attribute_key_is_unclassifiable(key, &tokens) || attribute_is_denied(key, &tokens) {
            removed = removed
                .checked_add(1)
                .ok_or(ScrubError::ScrubCountOverflow)?;
            increment(scrubbed_fields, 1)?;
            continue;
        }
        let mut child_ancestors = ancestors.to_vec();
        child_ancestors.extend(tokens);
        scrub_value(
            fields
                .get_mut("value")
                .ok_or(ScrubError::InvalidAttribute)?,
            &child_ancestors,
            scrubbed_fields,
        )?;
        kept.push(attribute);
    }
    *attributes = kept;
    Ok(removed)
}

fn scrub_kvlist(
    value: &mut Value,
    ancestors: &[String],
    scrubbed_fields: &mut u64,
) -> Result<(), ScrubError> {
    let fields = value.as_object_mut().ok_or(ScrubError::InvalidAttribute)?;
    if fields.len() != 1 {
        return Err(ScrubError::InvalidAttribute);
    }
    let values = fields
        .get_mut("values")
        .and_then(Value::as_array_mut)
        .ok_or(ScrubError::InvalidAttribute)?;
    let mut wrapper = Value::Array(std::mem::take(values));
    let _ = scrub_attributes(&mut wrapper, ancestors, scrubbed_fields)?;
    *values = wrapper
        .as_array_mut()
        .map(std::mem::take)
        .ok_or(ScrubError::InvalidAttribute)?;
    Ok(())
}

fn increment_dropped_attributes(
    fields: &mut Map<String, Value>,
    removed: u64,
) -> Result<(), ScrubError> {
    let previous = match fields.get("droppedAttributesCount") {
        Some(Value::Number(value)) => value
            .as_u64()
            .ok_or(ScrubError::InvalidDroppedAttributesCount)?,
        Some(_) => return Err(ScrubError::InvalidDroppedAttributesCount),
        None => return Ok(()),
    };
    let total = previous
        .checked_add(removed)
        .ok_or(ScrubError::ScrubCountOverflow)?;
    fields.insert("droppedAttributesCount".to_owned(), Value::from(total));
    Ok(())
}

fn attribute_key_is_unclassifiable(key: &str, tokens: &[String]) -> bool {
    tokens.is_empty() || key.bytes().any(|byte| byte.is_ascii_control())
}

fn attribute_is_denied(key: &str, tokens: &[String]) -> bool {
    key.contains('@') || deny_tokens(tokens)
}

fn object_field_is_denied(tokens: &[String], ancestors: &[String]) -> bool {
    deny_tokens(tokens)
        || (has_identifier(tokens) && has_identity_subject(ancestors))
        || (tokens.iter().any(|token| token == "name")
            && ancestors.iter().any(|token| token == "host"))
        || (tokens.iter().any(|token| token == "path")
            && ancestors
                .iter()
                .any(|token| matches!(token.as_str(), "home" | "workspace")))
}

fn deny_tokens(tokens: &[String]) -> bool {
    let contains = |needle: &str| tokens.iter().any(|token| token == needle);
    contains("email")
        || contains("hostname")
        || (contains("host") && contains("name"))
        || contains("cwd")
        || contains("workspace")
        || (contains("home")
            && tokens
                .iter()
                .any(|token| matches!(token.as_str(), "path" | "dir" | "directory" | "root")))
        || (has_identity_subject(tokens) && has_identifier(tokens))
        || (contains("request") && has_identifier(tokens))
        || tokens.iter().any(|token| {
            matches!(
                token.as_str(),
                "auth"
                    | "authorization"
                    | "authentication"
                    | "bearer"
                    | "cookie"
                    | "credential"
                    | "credentials"
                    | "header"
                    | "headers"
                    | "password"
                    | "passwd"
                    | "secret"
                    | "signature"
                    | "token"
            )
        })
        || ((contains("api") || contains("access") || contains("private")) && contains("key"))
}

fn has_identity_subject(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "user" | "account" | "organization" | "organisation" | "org" | "tenant"
        )
    })
}

fn has_identifier(tokens: &[String]) -> bool {
    tokens
        .iter()
        .any(|token| matches!(token.as_str(), "id" | "uuid" | "guid"))
}

fn key_tokens(key: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut previous_was_lower_or_digit = false;
    for character in key.chars() {
        if character.is_ascii_alphanumeric() {
            if character.is_ascii_uppercase() && previous_was_lower_or_digit && !current.is_empty()
            {
                tokens.push(std::mem::take(&mut current));
            }
            current.push(character.to_ascii_lowercase());
            previous_was_lower_or_digit =
                character.is_ascii_lowercase() || character.is_ascii_digit();
        } else {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            previous_was_lower_or_digit = false;
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn redact_sensitive_text(input: &str) -> Result<String, ScrubError> {
    let email_redacted = redact_emails(input)?;
    Ok(redact_absolute_paths(&email_redacted))
}

fn redact_emails(input: &str) -> Result<String, ScrubError> {
    let bytes = input.as_bytes();
    let mut ranges = Vec::new();
    for (at, byte) in bytes.iter().enumerate() {
        if *byte != b'@' {
            continue;
        }
        let mut start = at;
        while start > 0 && email_local_byte(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = at + 1;
        while end < bytes.len() && email_domain_byte(bytes[end]) {
            end += 1;
        }
        let local = &input[start..at];
        let domain = &input[at + 1..end];
        if local.is_empty() || !domain_is_named(domain) {
            continue;
        }
        if ranges
            .last()
            .is_none_or(|(_, previous_end)| *previous_end <= start)
        {
            ranges.push((start, end));
        }
    }
    if ranges.is_empty() {
        return Ok(input.to_owned());
    }
    let mut output = String::with_capacity(input.len());
    let mut copied = 0;
    for (start, end) in ranges {
        output.push_str(&input[copied..start]);
        output.push_str(REDACTED_EMAIL);
        copied = end;
    }
    output.push_str(&input[copied..]);
    Ok(output)
}

/// A named domain: labels separated by dots, none empty, no dash or dot at
/// either edge, ending in an alphabetic top-level label. The last condition is
/// what keeps a version in a stack trace (`@opentelemetry+api@1.9.0`) from
/// reading as a mail domain and being replaced inside the diagnostic.
fn domain_is_named(domain: &str) -> bool {
    if domain.is_empty() || domain.starts_with(['.', '-']) || domain.ends_with(['.', '-']) {
        return false;
    }
    let mut labels = domain.split('.');
    let Some(first) = labels.next() else {
        return false;
    };
    if first.is_empty() {
        return false;
    }
    let mut last = None;
    for label in labels {
        if label.is_empty() {
            return false;
        }
        last = Some(label);
    }
    last.is_some_and(|tld| tld.len() >= 2 && tld.bytes().all(|byte| byte.is_ascii_alphabetic()))
}

fn email_local_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'%' | b'+' | b'-')
}

fn email_domain_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')
}

fn redact_absolute_paths(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        let file_uri =
            bytes[index..].starts_with(b"file:///") && path_boundary_before(bytes, index);
        let unix_path = bytes[index] == b'/'
            && path_boundary_before(bytes, index)
            && bytes
                .get(index + 1)
                .is_some_and(|byte| *byte != b'/' && !path_delimiter(*byte));
        let windows_path = index + 2 < bytes.len()
            && bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && matches!(bytes[index + 2], b'\\' | b'/')
            && path_boundary_before(bytes, index);
        if !file_uri && !unix_path && !windows_path {
            index += 1;
            continue;
        }
        let start = index;
        index += if file_uri {
            "file:///".len()
        } else if windows_path {
            3
        } else {
            1
        };
        while index < bytes.len() && !path_delimiter(bytes[index]) {
            index += 1;
        }
        output.push_str(&input[copied..start]);
        output.push_str(REDACTED_PATH);
        copied = index;
    }
    output.push_str(&input[copied..]);
    output
}

fn path_boundary_before(bytes: &[u8], index: usize) -> bool {
    index == 0
        || bytes[index - 1].is_ascii_whitespace()
        || matches!(
            bytes[index - 1],
            b'"' | b'\'' | b'(' | b'[' | b'{' | b'=' | b':'
        )
}

fn path_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(byte, b'"' | b'\'' | b'<' | b'>' | b')' | b']' | b'}' | b',')
}

fn increment(counter: &mut u64, amount: u64) -> Result<(), ScrubError> {
    *counter = counter
        .checked_add(amount)
        .ok_or(ScrubError::ScrubCountOverflow)?;
    Ok(())
}

