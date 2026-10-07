use crate::daemon::ledger::projection::ProjectionSnapshot;
use circular_core::{Boundary, Ceilings, Value};
use circular_protocol::Partition;
use circular_protocol::declaration_payload::{Query, QueryPage, QueryResult, Rejected, Terminal};
use circular_protocol::rejection_code::RejectionReason;

pub(crate) const DEFAULT_PAGE_LIMIT: std::num::NonZeroUsize = engine::PRODUCT_STORE_PAGE_MAXIMUM;

pub(crate) struct OpenProjectionQuery {
    pub(super) correlation: u32,
    name: String,
    args: Value,
    since: Option<circular_protocol::replay_payload::LogCut>,
    snapshot: ProjectionSnapshot,
    last: Option<Value>,
    emitted: usize,
}

impl OpenProjectionQuery {
    pub(super) fn anchor(&self) -> &Value {
        &self.snapshot.anchor
    }
}

pub(super) fn query_rejection(reason: RejectionReason, message: &str) -> QueryResult {
    QueryResult::Rejected(rejection(reason, message))
}

pub(super) fn rejection(reason: RejectionReason, message: &str) -> Rejected {
    reason.reject(Partition::Query, message)
}

pub(crate) fn projection_page(
    correlation: u32,
    query: &Query,
    open: &mut Vec<OpenProjectionQuery>,
    capture: impl FnOnce() -> Result<ProjectionSnapshot, Rejected>,
) -> QueryResult {
    let invalid = |message| query_rejection(RejectionReason::Malformed, message);
    let limit = match query
        .page
        .as_ref()
        .map(|page| usize::try_from(page.limit.get()))
        .transpose()
    {
        Ok(limit) => limit.unwrap_or(DEFAULT_PAGE_LIMIT.get()),
        Err(_) => return invalid("query page limit exceeds this process"),
    };
    let supplied = query.page.as_ref().and_then(|page| page.cursor.as_ref());
    let existing = open
        .iter()
        .position(|entry| entry.correlation == correlation);
    let mut captured = None;
    let entry = match (existing, supplied) {
        (None, None) => {
            let snapshot = match capture() {
                Ok(snapshot) => snapshot,
                Err(rejection) => return QueryResult::Rejected(rejection),
            };
            captured = Some(OpenProjectionQuery {
                correlation,
                name: query.name.clone(),
                args: query.args.clone(),
                since: query.since.clone(),
                snapshot,
                last: None,
                emitted: 0,
            });
            captured.as_mut().expect("captured query")
        }
        (Some(index), Some(cursor)) => {
            let entry = &mut open[index];
            if entry.name != query.name
                || entry.args != query.args
                || (query.since.is_some() && entry.since != query.since)
                || entry.last.as_ref() != Some(cursor)
            {
                return invalid(
                    "query cursor differs from the retained registration, arguments or immutable reference",
                );
            }
            entry
        }
        (Some(_), None) => return invalid("query restarted a live correlation without its cursor"),
        (None, Some(_)) => return invalid("query cursor has no retained immutable snapshot"),
    };
    let base = entry.emitted;
    if let Some(mut tail) = entry.snapshot.records_tail.take() {
        match tail.fill(&mut entry.snapshot.records, limit.saturating_add(1)) {
            Ok(false) => entry.snapshot.records_tail = Some(tail),
            Ok(true) => {}
            Err(message) => {
                if let Some(index) = existing {
                    open.remove(index);
                }
                return query_rejection(RejectionReason::Unresolved, &message);
            }
        }
    }
    let cursor_for = |end: usize| {
        Value::object([
            ("anchor", entry.snapshot.anchor.clone()),
            ("domain", Value::String(entry.name.clone())),
            (
                "position",
                Value::bytes(entry.snapshot.records[end - 1].witness.as_bytes().to_vec()),
            ),
        ])
        .expect("canonical Cursor fields")
    };
    let offset = 0;
    let maximum = limit.min(entry.snapshot.records.len());
    let mut items = Vec::new();
    let mut bytes = 0;
    for row in &entry.snapshot.records[offset..maximum] {
        let value = match row.value() {
            Ok(value) => value,
            Err(reason) => {
                if let Some(index) = existing {
                    open.remove(index);
                }
                return query_rejection(RejectionReason::Unresolved, &reason);
            }
        };
        let size = match circular_core::encode(&value, Ceilings::for_boundary(Boundary::Wire)) {
            Ok(encoded) => encoded.len(),
            Err(error) => {
                if items.is_empty() {
                    if let Some(index) = existing {
                        open.remove(index);
                    }
                    return super::query_result_encoding_failed_with_reason(&error);
                }
                break;
            }
        };
        if !items.is_empty() && bytes + size > Ceilings::for_boundary(Boundary::Wire).max_bytes() {
            break;
        }
        bytes += size;
        items.push(value);
    }
    let page_for = |items: &[Value]| {
        let end = items.len();
        QueryResult::Page(QueryPage {
            cut: entry.snapshot.cut.clone(),
            folded_from: entry.snapshot.folded_from.clone(),
            anchor: entry.snapshot.anchor.clone(),
            items: items.to_vec(),
            reached: entry.snapshot.reached(base + end),
            terminal: if end < entry.snapshot.records.len() {
                Terminal::More(cursor_for(end))
            } else {
                Terminal::Complete
            },
        })
    };
    let mut result = page_for(&items);
    while result
        .encode(Ceilings::for_boundary(Boundary::Wire))
        .is_err()
        && items.len() > 1
    {
        items.pop();
        result = page_for(&items);
    }
    if let Err(error) = result.encode(Ceilings::for_boundary(Boundary::Wire)) {
        if let Some(index) = existing {
            open.remove(index);
        }
        return super::query_result_encoding_failed_with_reason(&error);
    }
    let QueryResult::Page(page) = &result else {
        unreachable!()
    };
    if let Terminal::More(cursor) = &page.terminal {
        let cursor = cursor.clone();
        let emitted = page.items.len();
        entry.last = Some(cursor);
        entry.snapshot.records.drain(..emitted);
        entry.emitted = base + emitted;
        if existing.is_none() {
            open.push(captured.expect("retained immutable query"));
        }
    } else if let Some(index) = existing {
        open.remove(index);
    }
    result
}

