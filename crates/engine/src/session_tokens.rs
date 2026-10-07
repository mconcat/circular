use circular_protocol::{SESSION_TOKEN_BYTES, SessionTokenSource};

/// No clock, process id, or deterministic fallback enters token generation.
pub struct OsSessionTokenSource;

impl SessionTokenSource for OsSessionTokenSource {
    type Error = getrandom::Error;

    fn draw(&mut self) -> Result<[u8; SESSION_TOKEN_BYTES], Self::Error> {
        let mut bytes = [0; SESSION_TOKEN_BYTES];
        getrandom::fill(&mut bytes)?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_protocol::SessionTokenIssuer;
    use std::num::NonZeroUsize;

    #[test]
    fn os_tokens_are_full_width_distinct_and_retire_without_printing_bytes() {
        let mut issuer = SessionTokenIssuer::new();
        let mut source = OsSessionTokenSource;
        let first = issuer
            .issue(&mut source, NonZeroUsize::new(2).unwrap())
            .unwrap();
        let second = issuer
            .issue(&mut source, NonZeroUsize::new(2).unwrap())
            .unwrap();
        assert_eq!(first.as_bytes().len(), 32);
        assert_ne!(first, second);
        assert_eq!(format!("{first:?}"), "SessionToken(..)");
        assert!(issuer.retire(&first));
        assert!(!issuer.is_live(&first));
        assert!(issuer.is_live(&second));
    }
}
