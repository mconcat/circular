use std::fmt;

/// Compile-time artifact identity, separate from compatibility axes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildIdentity {
    pub program: &'static str,
    pub version: &'static str,
    pub git_sha: &'static str,
    pub git_dirty: bool,
    pub commit_date: &'static str,
    pub target: &'static str,
}

/// The workspace package version every participating binary inherits.
pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
#[must_use]
pub const fn build_identity(program: &'static str) -> BuildIdentity {
    BuildIdentity {
        program,
        version: PACKAGE_VERSION,
        git_sha: env!("BUILD_GIT_SHA"),
        git_dirty: matches!(env!("BUILD_GIT_DIRTY").as_bytes(), b"true"),
        commit_date: env!("BUILD_COMMIT_DATE"),
        target: env!("BUILD_TARGET"),
    }
}

impl fmt::Display for BuildIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} ({}{} {} {})",
            self.program,
            self.version,
            self.git_sha,
            if self.git_dirty { "-dirty" } else { "" },
            self.commit_date,
            self.target
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_renders_clean_dirty_and_unknown() {
        let mut identity = BuildIdentity {
            program: "circular-daemon",
            version: "0.0.0",
            git_sha: "abc123def456",
            git_dirty: false,
            commit_date: "2026-09-06",
            target: "aarch64-apple-darwin",
        };
        assert_eq!(
            identity.to_string(),
            "circular-daemon 0.0.0 (abc123def456 2026-09-06 aarch64-apple-darwin)"
        );
        identity.git_dirty = true;
        assert_eq!(
            identity.to_string(),
            "circular-daemon 0.0.0 (abc123def456-dirty 2026-09-06 aarch64-apple-darwin)"
        );
        identity.git_sha = "unknown";
        identity.commit_date = "unknown";
        identity.git_dirty = false;
        assert_eq!(
            identity.to_string(),
            "circular-daemon 0.0.0 (unknown unknown aarch64-apple-darwin)"
        );
    }

    #[test]
    fn compiled_identity_has_the_expected_shape() {
        let identity = build_identity("circular-daemon");
        assert!(
            identity.git_sha == "unknown"
                || (identity.git_sha.len() == 12
                    && identity
                        .git_sha
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        );
        assert!(
            identity.commit_date == "unknown"
                || (identity.commit_date.len() == 10
                    && identity.commit_date.bytes().enumerate().all(
                        |(i, b)| if i == 4 || i == 7 {
                            b == b'-'
                        } else {
                            b.is_ascii_digit()
                        }
                    ))
        );
        let (core, pre_release) = match identity.version.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (identity.version, None),
        };
        assert_eq!(core.split('.').count(), 3);
        assert!(
            core.split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        );
        if let Some(pre) = pre_release {
            assert!(pre.split('.').all(|part| !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')));
        }
        assert!(!identity.target.is_empty());
        assert!(!identity.target.contains(char::is_whitespace));
        assert_eq!(identity.to_string().lines().count(), 1);
    }

    #[test]
    fn all_three_programs_render_four_axes_in_declaration_order() {
        for program in ["circular-daemon", "circular-agent", "circular-client"] {
            let output = crate::owner_local_version(program);
            let lines: Vec<_> = output.lines().collect();
            assert_eq!(lines[0], build_identity(program).to_string());
            assert_eq!(
                &lines[1..],
                &[
                    "authoring-api: undeclared",
                    "actor-spec: undeclared",
                    "wire: protocol 1, framing 1",
                    "record: format 1, payload 1",
                ]
            );
            assert!(output.ends_with('\n'));
        }
    }
}
