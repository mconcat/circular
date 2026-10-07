
use crate::{Cursor, PageEnd, PageStep};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryPhase {
    AwaitingFirst,
    AwaitingContinuation,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum QueryState<A, D, P> {
    AwaitingFirst,
    AwaitingContinuation(Cursor<A, D, P>),
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryCursorError {
    WrongRegistration,
    UnexpectedFirstPage,
    UnexpectedContinuation,
    CursorMismatch,
    CursorAnchorMismatch,
    CursorDomainMismatch,
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryCursorFsm<R, A, D, P> {
    registration: R,
    anchor: A,
    domain: D,
    state: QueryState<A, D, P>,
}

impl<R, A, D, P> QueryCursorFsm<R, A, D, P> {
    #[must_use]
    pub const fn new(registration: R, anchor: A, domain: D) -> Self {
        Self {
            registration,
            anchor,
            domain,
            state: QueryState::AwaitingFirst,
        }
    }

    #[must_use]
    pub const fn registration(&self) -> &R {
        &self.registration
    }

    #[must_use]
    pub const fn anchor(&self) -> &A {
        &self.anchor
    }

    #[must_use]
    pub const fn domain(&self) -> &D {
        &self.domain
    }

    #[must_use]
    pub const fn phase(&self) -> QueryPhase {
        match self.state {
            QueryState::AwaitingFirst => QueryPhase::AwaitingFirst,
            QueryState::AwaitingContinuation(_) => QueryPhase::AwaitingContinuation,
            QueryState::Closed => QueryPhase::Closed,
        }
    }

    pub fn page<L, E>(
        &mut self,
        registration: &R,
        request: &PageStep<L, Cursor<A, D, P>>,
        end: PageEnd<Cursor<A, D, P>, E>,
    ) -> Result<(), QueryCursorError>
    where
        R: Eq,
        A: Clone + Eq,
        D: Clone + Eq,
        P: Clone + Eq,
    {
        if registration != &self.registration {
            return Err(QueryCursorError::WrongRegistration);
        }
        match (&self.state, request) {
            (QueryState::AwaitingFirst, PageStep::First { .. }) => {}
            (QueryState::AwaitingFirst, PageStep::Continue { .. }) => {
                return Err(QueryCursorError::UnexpectedContinuation);
            }
            (QueryState::AwaitingContinuation(_), PageStep::First { .. }) => {
                return Err(QueryCursorError::UnexpectedFirstPage);
            }
            (QueryState::AwaitingContinuation(expected), PageStep::Continue { cursor, .. })
                if cursor == expected => {}
            (QueryState::AwaitingContinuation(_), PageStep::Continue { .. }) => {
                return Err(QueryCursorError::CursorMismatch);
            }
            (QueryState::Closed, _) => return Err(QueryCursorError::Closed),
        }

        self.state = match end {
            PageEnd::More { next } => {
                if next.anchor() != &self.anchor {
                    return Err(QueryCursorError::CursorAnchorMismatch);
                }
                if next.domain() != &self.domain {
                    return Err(QueryCursorError::CursorDomainMismatch);
                }
                QueryState::AwaitingContinuation(next)
            }
            PageEnd::Complete | PageEnd::Diagnostic { .. } => QueryState::Closed,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor<'a>(anchor: &'a str, domain: &'a str, position: u8) -> Cursor<&'a str, &'a str, u8> {
        Cursor::new(anchor, domain, position)
    }

    #[test]
    fn page_transcript_requires_exact_registration_anchor_domain_and_cursor() {
        let mut query = QueryCursorFsm::new("structure-of", "revision-a", "actor-order");
        let first = PageStep::First { limit: 2_u8 };
        let next = cursor("revision-a", "actor-order", 2);
        query
            .page(
                &"structure-of",
                &first,
                PageEnd::<_, ()>::More { next: next.clone() },
            )
            .expect("first page");
        assert_eq!(query.phase(), QueryPhase::AwaitingContinuation);

        let wrong_registration = PageStep::Continue {
            limit: 2,
            cursor: next.clone(),
        };
        assert_eq!(
            query.page(
                &"other-query",
                &wrong_registration,
                PageEnd::<_, ()>::Complete
            ),
            Err(QueryCursorError::WrongRegistration)
        );
        assert_eq!(query.phase(), QueryPhase::AwaitingContinuation);

        let stale = PageStep::Continue {
            limit: 2,
            cursor: cursor("revision-a", "actor-order", 1),
        };
        assert_eq!(
            query.page(&"structure-of", &stale, PageEnd::<_, ()>::Complete),
            Err(QueryCursorError::CursorMismatch)
        );

        let continuation = PageStep::Continue {
            limit: 7,
            cursor: next,
        };
        query
            .page(&"structure-of", &continuation, PageEnd::<_, ()>::Complete)
            .expect("exact continuation");
        assert_eq!(query.phase(), QueryPhase::Closed);
        assert_eq!(
            query.page(
                &"structure-of",
                &PageStep::First { limit: 1 },
                PageEnd::<Cursor<&str, &str, u8>, ()>::Complete
            ),
            Err(QueryCursorError::Closed)
        );
    }

    #[test]
    fn invalid_next_cursor_does_not_advance_the_stream() {
        let mut query = QueryCursorFsm::new("records", "upto-9", "record-position");
        let first = PageStep::First { limit: 2_u8 };
        assert_eq!(
            query.page(
                &"records",
                &first,
                PageEnd::<_, ()>::More {
                    next: cursor("upto-8", "record-position", 2)
                }
            ),
            Err(QueryCursorError::CursorAnchorMismatch)
        );
        assert_eq!(query.phase(), QueryPhase::AwaitingFirst);

        assert_eq!(
            query.page(
                &"records",
                &first,
                PageEnd::<_, ()>::More {
                    next: cursor("upto-9", "other-domain", 2)
                }
            ),
            Err(QueryCursorError::CursorDomainMismatch)
        );
        assert_eq!(query.phase(), QueryPhase::AwaitingFirst);
    }

    #[test]
    fn diagnostic_is_a_terminal_value_not_a_silent_stop() {
        let mut query = QueryCursorFsm::new("records", "upto-9", "record-position");
        query
            .page(
                &"records",
                &PageStep::First { limit: 4_u8 },
                PageEnd::<Cursor<&str, &str, u8>, _>::Diagnostic { code: "stale" },
            )
            .expect("diagnostic terminal");
        assert_eq!(query.phase(), QueryPhase::Closed);
    }

    #[test]
    fn page_limit_does_not_change_cursor_acceptance() {
        for limit in 1_u8..=8 {
            let mut query = QueryCursorFsm::new("records", "anchor", "domain");
            query
                .page(
                    &"records",
                    &PageStep::First { limit },
                    PageEnd::<Cursor<&str, &str, u8>, ()>::Complete,
                )
                .expect("every injected positive ceiling has the same terminal transition");
            assert_eq!(query.phase(), QueryPhase::Closed);
        }
    }
}
