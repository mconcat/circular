
use std::fmt;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ResourceName(Box<str>);

impl ResourceName {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() || raw.chars().any(char::is_control) {
            return None;
        }
        Some(Self(raw.into()))
    }
}

impl fmt::Display for ResourceName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::ResourceName;

    #[test]
    fn names_are_nonempty_control_free_spellings_kept_verbatim() {
        let name = ResourceName::parse("slack-bot").expect("a plain spelling is a name");
        assert_eq!(name.to_string(), "slack-bot");
        let spaced = ResourceName::parse("  Alerts  ").expect("a space is not a control character");
        assert_eq!(spaced.to_string(), "  Alerts  ");
        assert_eq!(ResourceName::parse(""), None);
        assert_eq!(ResourceName::parse("two\nlines"), None);
        assert_eq!(ResourceName::parse("tab\there"), None);
    }
}
