
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use subtle::{Choice, ConstantTimeEq};

pub const SESSION_TOKEN_BITS: usize = 256;

pub const SESSION_TOKEN_BYTES: usize = SESSION_TOKEN_BITS / 8;

pub const REVISION_DIGEST_BYTES: usize = 32;

#[derive(Clone, Copy, Eq)]
pub struct SessionToken([u8; SESSION_TOKEN_BYTES]);

impl ConstantTimeEq for SessionToken {
    fn ct_eq(&self, other: &Self) -> Choice {
        self.0.ct_eq(&other.0)
    }
}

impl PartialEq for SessionToken {
    fn eq(&self, other: &Self) -> bool {
        bool::from(self.ct_eq(other))
    }
}

impl SessionToken {
    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, SessionTokenWidthMismatch> {
        <[u8; SESSION_TOKEN_BYTES]>::try_from(bytes)
            .map(Self)
            .map_err(|_| SessionTokenWidthMismatch { got: bytes.len() })
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; SESSION_TOKEN_BYTES] {
        &self.0
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionToken(..)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionTokenWidthMismatch {
    got: usize,
}

impl fmt::Display for SessionTokenWidthMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "session token must be {SESSION_TOKEN_BYTES} bytes; received {} bytes",
            self.got
        )
    }
}

impl std::error::Error for SessionTokenWidthMismatch {}

pub trait SessionTokenSource {
    type Error;
    fn draw(&mut self) -> Result<[u8; SESSION_TOKEN_BYTES], Self::Error>;
}

/// Failure to obtain a fresh token; neither branch publishes a candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionTokenIssueError<E> {
    Source(E),
    Exhausted(SessionTokensExhausted),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionTokenIssuer {
    live: Vec<SessionToken>,
}

impl SessionTokenIssuer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn issue<S: SessionTokenSource>(
        &mut self,
        source: &mut S,
        attempts: NonZeroUsize,
    ) -> Result<SessionToken, SessionTokenIssueError<S::Error>> {
        for _ in 0..attempts.get() {
            let candidate = SessionToken(source.draw().map_err(SessionTokenIssueError::Source)?);
            if !self.is_live(&candidate) {
                self.live.push(candidate);
                return Ok(candidate);
            }
        }
        Err(SessionTokenIssueError::Exhausted(SessionTokensExhausted {
            attempts: attempts.get(),
        }))
    }

    #[must_use]
    pub fn is_live(&self, token: &SessionToken) -> bool {
        bool::from(
            self.live
                .iter()
                .fold(Choice::from(0), |found, live| found | live.ct_eq(token)),
        )
    }

    pub fn retire(&mut self, token: &SessionToken) -> bool {
        let before = self.live.len();
        self.live.retain(|live| !bool::from(live.ct_eq(token)));
        self.live.len() != before
    }

    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionTokensExhausted {
    attempts: usize,
}

impl fmt::Display for SessionTokensExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "failed to obtain an unused session token within {} redraws",
            self.attempts
        )
    }
}

impl std::error::Error for SessionTokensExhausted {}

pub trait RevisionKind {}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TopologyRevision;

impl RevisionKind for TopologyRevision {}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AuthoringRevision;

impl RevisionKind for AuthoringRevision {}

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RevisionDigest<K> {
    bytes: [u8; REVISION_DIGEST_BYTES],
    kind: PhantomData<fn() -> K>,
}

impl<K: RevisionKind> RevisionDigest<K> {
    pub fn try_from_bytes(bytes: &[u8]) -> Result<Self, RevisionWidthMismatch> {
        <[u8; REVISION_DIGEST_BYTES]>::try_from(bytes)
            .map(|bytes| Self {
                bytes,
                kind: PhantomData,
            })
            .map_err(|_| RevisionWidthMismatch { got: bytes.len() })
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; REVISION_DIGEST_BYTES] {
        &self.bytes
    }
}

impl<K: RevisionKind> fmt::Debug for RevisionDigest<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:", std::any::type_name::<K>())?;
        for byte in &self.bytes {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionWidthMismatch {
    got: usize,
}

impl fmt::Display for RevisionWidthMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "revision digest must be {REVISION_DIGEST_BYTES} bytes; received {} bytes",
            self.got
        )
    }
}

impl std::error::Error for RevisionWidthMismatch {}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sequence(Vec<[u8; SESSION_TOKEN_BYTES]>);

    impl SessionTokenSource for Sequence {
        type Error = std::convert::Infallible;
        fn draw(&mut self) -> Result<[u8; SESSION_TOKEN_BYTES], Self::Error> {
            Ok(self.0.remove(0))
        }
    }

    fn token(seed: u8) -> [u8; SESSION_TOKEN_BYTES] {
        [seed; SESSION_TOKEN_BYTES]
    }

    fn attempts(count: usize) -> NonZeroUsize {
        NonZeroUsize::new(count).expect("positive")
    }

    #[test]
    fn only_the_exact_width_becomes_a_token() {
        assert_eq!(SESSION_TOKEN_BITS, 256);
        assert!(SessionToken::try_from_bytes(&[7; 32]).is_ok());
        assert!(SessionToken::try_from_bytes(&[7; 16]).is_err());
        for wrong in [SESSION_TOKEN_BYTES - 1, SESSION_TOKEN_BYTES + 1, 0] {
            assert_eq!(
                SessionToken::try_from_bytes(&vec![7; wrong]),
                Err(SessionTokenWidthMismatch { got: wrong }),
                "width {wrong} is not a token"
            );
        }
    }

    #[test]
    fn equality_is_the_whole_width_and_a_shared_prefix_is_not_enough() {
        let mut near = token(0);
        near[SESSION_TOKEN_BYTES - 1] = 1;
        let base = SessionToken::try_from_bytes(&token(0)).expect("width");
        let tail = SessionToken::try_from_bytes(&near).expect("width");

        assert_ne!(
            base, tail,
            "differing only in the last byte is a different token"
        );
        assert_eq!(
            base,
            SessionToken::try_from_bytes(&token(0)).expect("width"),
            "equal only when every byte is equal"
        );
    }

    #[test]
    fn a_colliding_candidate_is_redrawn_and_never_leaves_the_issuer() {
        let mut issuer = SessionTokenIssuer::new();
        let mut source = Sequence(vec![token(1), token(1), token(1), token(2)]);

        let first = issuer.issue(&mut source, attempts(4)).expect("first issue");
        let second = issuer
            .issue(&mut source, attempts(4))
            .expect("issue after redraw");

        assert_ne!(first, second);
        assert_eq!(
            issuer.live_count(),
            2,
            "the colliding candidate did not enter the set"
        );
        assert!(issuer.is_live(&first) && issuer.is_live(&second));
    }

    #[test]
    fn exhaustion_refuses_the_new_session_and_leaves_the_live_set_alone() {
        let mut issuer = SessionTokenIssuer::new();
        let mut source = Sequence(vec![token(9), token(9), token(9)]);
        let held = issuer.issue(&mut source, attempts(1)).expect("first issue");

        assert_eq!(
            issuer.issue(&mut source, attempts(2)),
            Err(SessionTokenIssueError::Exhausted(SessionTokensExhausted {
                attempts: 2
            }))
        );
        assert_eq!(
            issuer.live_count(),
            1,
            "exhaustion does not touch a live session"
        );
        assert!(issuer.is_live(&held));
    }

    #[test]
    fn source_failure_does_not_publish_a_token_or_retire_existing_tokens() {
        struct Failed;
        impl SessionTokenSource for Failed {
            type Error = &'static str;
            fn draw(&mut self) -> Result<[u8; SESSION_TOKEN_BYTES], Self::Error> {
                Err("entropy unavailable")
            }
        }
        let mut issuer = SessionTokenIssuer::new();
        let held = issuer
            .issue(&mut Sequence(vec![token(1)]), attempts(1))
            .unwrap();
        assert_eq!(
            issuer.issue(&mut Failed, attempts(2)),
            Err(SessionTokenIssueError::Source("entropy unavailable"))
        );
        assert_eq!(issuer.live_count(), 1);
        assert!(issuer.is_live(&held));
    }

    #[test]
    fn a_retired_token_leaves_the_live_set_and_its_value_can_be_drawn_again() {
        let mut issuer = SessionTokenIssuer::new();
        let mut source = Sequence(vec![token(3), token(3)]);
        let first = issuer.issue(&mut source, attempts(1)).expect("issue");

        assert!(issuer.retire(&first));
        assert!(!issuer.is_live(&first));
        let reissued = issuer
            .issue(&mut source, attempts(1))
            .expect("a dead value is drawn again");
        assert_eq!(first, reissued);
    }

    #[test]
    fn a_token_does_not_print_its_value() {
        let token = SessionToken::try_from_bytes(&token(0xab)).expect("width");
        let shown = format!("{token:?}");
        assert_eq!(shown, "SessionToken(..)");
        assert!(!shown.contains("ab"));
    }

    #[test]
    fn only_the_exact_width_becomes_a_revision() {
        assert!(
            RevisionDigest::<TopologyRevision>::try_from_bytes(&[0; REVISION_DIGEST_BYTES]).is_ok()
        );
        for wrong in [REVISION_DIGEST_BYTES - 1, REVISION_DIGEST_BYTES + 1] {
            assert_eq!(
                RevisionDigest::<TopologyRevision>::try_from_bytes(&vec![0; wrong]),
                Err(RevisionWidthMismatch { got: wrong })
            );
        }
    }

    #[test]
    fn equality_compares_every_byte_of_a_revision() {
        let base = RevisionDigest::<TopologyRevision>::try_from_bytes(&[5; REVISION_DIGEST_BYTES])
            .expect("width");
        for position in [0, REVISION_DIGEST_BYTES / 2, REVISION_DIGEST_BYTES - 1] {
            let mut altered = [5; REVISION_DIGEST_BYTES];
            altered[position] = 6;
            let other =
                RevisionDigest::<TopologyRevision>::try_from_bytes(&altered).expect("width");
            assert_ne!(
                base, other,
                "differing only in byte {position} is different"
            );
        }
    }

    #[test]
    fn the_provisional_widths_satisfy_their_inequalities() {
        let (q, n, epsilon_g) = (40_u32, 12_u32, 64_u32);
        assert!(
            q + n + epsilon_g <= SESSION_TOKEN_BITS as u32,
            "log2(q·n/ε_g) must not exceed the width"
        );
        let (big_n, epsilon) = (40_u32, 64_u32);
        let b = (REVISION_DIGEST_BYTES * 8) as u32;
        assert!(
            2 * big_n <= b + 1 - epsilon,
            "the birthday bound must stay below ε"
        );
    }
}
