use std::fmt;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Segment {
    Key(String),
    Index(u64),
}

impl Segment {
    #[must_use]
    pub fn key(key: impl Into<String>) -> Self {
        Self::Key(key.into())
    }

    #[must_use]
    pub const fn index(index: u64) -> Self {
        Self::Index(index)
    }

    #[must_use]
    pub fn as_key(&self) -> Option<&str> {
        match self {
            Self::Key(key) => Some(key),
            Self::Index(_) => None,
        }
    }

    #[must_use]
    pub const fn as_index(&self) -> Option<u64> {
        match self {
            Self::Index(index) => Some(*index),
            Self::Key(_) => None,
        }
    }
}

impl fmt::Display for Segment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(key) => write!(formatter, ".{key}"),
            Self::Index(index) => write!(formatter, "[{index}]"),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ValuePath<R> {
    root: R,
    segments: Vec<Segment>,
}

impl<R> ValuePath<R> {
    #[must_use]
    pub fn new(root: R, segments: impl IntoIterator<Item = Segment>) -> Self {
        Self {
            root,
            segments: segments.into_iter().collect(),
        }
    }

    #[must_use]
    pub const fn root_kind(&self) -> &R {
        &self.root
    }

    #[must_use]
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    #[must_use]
    pub fn join(mut self, segment: Segment) -> Self {
        self.segments.push(segment);
        self
    }

    #[must_use]
    pub fn join_key(self, key: impl Into<String>) -> Self {
        self.join(Segment::key(key))
    }

    #[must_use]
    pub fn join_index(self, index: u64) -> Self {
        self.join(Segment::index(index))
    }

    #[must_use]
    pub fn extend<S>(mut self, relative: ValuePath<S>) -> Self {
        let (_, segments) = relative.into_parts();
        self.segments.extend(segments);
        self
    }

    #[must_use]
    pub fn strip_prefix<S, T>(&self, prefix: &ValuePath<S>, root: T) -> Option<ValuePath<T>> {
        let prefix = prefix.segments();
        if !self.segments.starts_with(prefix) {
            return None;
        }
        Some(ValuePath::new(
            root,
            self.segments[prefix.len()..].iter().cloned(),
        ))
    }

    #[must_use]
    pub fn starts_with<S>(&self, prefix: &ValuePath<S>) -> bool {
        self.segments.starts_with(prefix.segments())
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.segments.len()
    }

    #[must_use]
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    #[must_use]
    pub fn into_parts(self) -> (R, Vec<Segment>) {
        (self.root, self.segments)
    }
}

impl<R: fmt::Display> fmt::Display for ValuePath<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.root.fmt(formatter)?;
        self.segments
            .iter()
            .try_for_each(|segment| segment.fmt(formatter))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConfigRoot;

impl fmt::Display for ConfigRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("config")
    }
}

pub type ConfigPath = ValuePath<ConfigRoot>;

impl ValuePath<ConfigRoot> {
    #[must_use]
    pub const fn root() -> Self {
        Self {
            root: ConfigRoot,
            segments: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ElementRoot;

pub type ElementPath = ValuePath<ElementRoot>;

impl ValuePath<ElementRoot> {
    #[must_use]
    pub const fn element() -> Self {
        Self {
            root: ElementRoot,
            segments: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigPath, ElementPath, ElementRoot, Segment};

    #[test]
    fn config_path_display_is_the_one_human_spelling() {
        assert_eq!(ConfigPath::root().to_string(), "config");
        assert_eq!(
            ConfigPath::root()
                .join_key("items")
                .join_index(2)
                .join_key("name")
                .to_string(),
            "config.items[2].name"
        );
        assert_eq!(Segment::key("0").to_string(), ".0");
        assert_eq!(Segment::index(0).to_string(), "[0]");
    }

    #[test]
    fn config_root_is_a_const_empty_exact_path() {
        const ROOT: ConfigPath = ConfigPath::root();

        assert_eq!(ROOT.segments(), []);
    }

    #[test]
    fn key_zero_and_index_zero_remain_distinct() {
        let key = ConfigPath::root().join_key("0");
        let index = ConfigPath::root().join_index(0);

        assert_ne!(key, index);
        assert_eq!(key.segments(), [Segment::key("0")]);
        assert_eq!(index.segments(), [Segment::index(0)]);
    }

    #[test]
    fn path_preserves_root_and_segment_order() {
        let path = ConfigPath::root()
            .join_key("items")
            .join_index(2)
            .join_key("name");

        assert_eq!(
            path.segments(),
            [
                Segment::key("items"),
                Segment::index(2),
                Segment::key("name"),
            ]
        );
    }

    fn container() -> ConfigPath {
        ConfigPath::root().join_key("items").join_index(2)
    }

    #[test]
    fn extending_keeps_the_base_root_and_appends_in_order() {
        let joined = container().extend(ElementPath::element().join_key("name"));

        assert_eq!(joined.root_kind(), &super::ConfigRoot);
        assert_eq!(
            joined.segments(),
            [
                Segment::key("items"),
                Segment::index(2),
                Segment::key("name"),
            ]
        );
    }

    #[test]
    fn extending_by_the_element_root_is_identity() {
        assert_eq!(container().extend(ElementPath::element()), container());
    }

    #[test]
    fn extending_is_associative() {
        let first = ElementPath::element().join_key("a");
        let second = ElementPath::element().join_index(7);

        let left = container().extend(first.clone()).extend(second.clone());
        let right = container().extend(first.extend(second));

        assert_eq!(left, right);
    }

    #[test]
    fn stripping_undoes_extending() {
        for inside in [
            ElementPath::element(),
            ElementPath::element().join_key("name"),
            ElementPath::element().join_key("tags").join_index(0),
        ] {
            let full = container().extend(inside.clone());
            assert_eq!(
                full.strip_prefix(&container(), ElementRoot),
                Some(inside.clone()),
                "{inside:?}"
            );
        }
    }

    #[test]
    fn stripping_a_non_prefix_yields_nothing() {
        let elsewhere = ConfigPath::root().join_key("other");
        let full = container().join_key("name");

        assert_eq!(full.strip_prefix(&elsewhere, ElementRoot), None);
        assert!(!full.starts_with(&elsewhere));
        assert!(full.starts_with(&container()));
    }

    #[test]
    fn depth_and_rootness_agree() {
        assert!(ConfigPath::root().is_root());
        assert_eq!(ConfigPath::root().depth(), 0);
        assert!(ElementPath::element().is_root());

        let path = container();
        assert!(!path.is_root());
        assert_eq!(path.depth(), 2);
    }
}
