//! The directory permission policy shared by invocation and socket validation.

use std::fmt;

/// OwnerLocal state roots require all owner permissions and no special bits.
pub const OWNER_ROOT_MODE: u32 = 0o700;
pub(crate) const PERMISSION_AND_SPECIAL_BITS: u32 = 0o7777;

/// A state root's permission and special bits differ from exact mode `0700`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnerRootModeMismatch {
    pub actual: u32,
}

impl fmt::Display for OwnerRootModeMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "owner-local state directory mode {:#o} must be exactly {OWNER_ROOT_MODE:#o} with no special bits",
            self.actual
        )
    }
}

impl std::error::Error for OwnerRootModeMismatch {}

/// Validates a raw filesystem mode without discarding setuid, setgid or sticky.
///
/// This checks one observation only. Callers must still re-read metadata and
/// validate again at every existing TOCTOU boundary.
pub fn validate_owner_root_mode(mode: u32) -> Result<(), OwnerRootModeMismatch> {
    let actual = mode & PERMISSION_AND_SPECIAL_BITS;
    if actual != OWNER_ROOT_MODE {
        return Err(OwnerRootModeMismatch { actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_0700_is_accepted_regardless_of_file_type_bits() {
        for permissions in 0..=0o7777 {
            for file_type in [0, 0o040000] {
                let result = validate_owner_root_mode(file_type | permissions);
                if permissions == 0o700 {
                    assert_eq!(result, Ok(()));
                } else {
                    assert_eq!(
                        result,
                        Err(OwnerRootModeMismatch {
                            actual: permissions
                        })
                    );
                }
            }
        }
    }
}
