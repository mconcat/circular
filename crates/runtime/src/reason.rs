
use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Suppression;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeadLettering;

pub struct Reason<K>(Box<str>, PhantomData<K>);

pub type SuppressionReason = Reason<Suppression>;

pub type DeclaredReason = Reason<DeadLettering>;

impl<K> Reason<K> {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl<K> Clone for Reason<K> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<K> fmt::Debug for Reason<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Reason").field(&self.0).finish()
    }
}

impl<K> PartialEq for Reason<K> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<K> Eq for Reason<K> {}

impl<K> PartialOrd for Reason<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K> Ord for Reason<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl<K> std::hash::Hash for Reason<K> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<K> fmt::Display for Reason<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReasonDeclError {
    Empty,
    Duplicate(Box<str>),
}

impl fmt::Display for ReasonDeclError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("reason declaration is empty"),
            Self::Duplicate(name) => write!(formatter, "reason name {name:?} is declared twice"),
        }
    }
}

impl std::error::Error for ReasonDeclError {}

#[derive(Debug)]
pub struct ReasonDecl<K> {
    names: BTreeSet<Box<str>>,
    kind: PhantomData<K>,
}

impl<K> Clone for ReasonDecl<K> {
    fn clone(&self) -> Self {
        Self {
            names: self.names.clone(),
            kind: PhantomData,
        }
    }
}

impl<K> PartialEq for ReasonDecl<K> {
    fn eq(&self, other: &Self) -> bool {
        self.names == other.names
    }
}

impl<K> Eq for ReasonDecl<K> {}

impl<K> ReasonDecl<K> {
    pub fn try_from_names<S>(names: impl IntoIterator<Item = S>) -> Result<Self, ReasonDeclError>
    where
        S: Into<Box<str>>,
    {
        let mut set = BTreeSet::new();
        for name in names {
            let name = name.into();
            if !set.insert(name.clone()) {
                return Err(ReasonDeclError::Duplicate(name));
            }
        }
        if set.is_empty() {
            return Err(ReasonDeclError::Empty);
        }
        Ok(Self {
            names: set,
            kind: PhantomData,
        })
    }

    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<Reason<K>> {
        self.names
            .get(name)
            .map(|declared| Reason(declared.clone(), PhantomData))
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(AsRef::as_ref)
    }
}

#[cfg(test)]
mod tests {
    use super::{ReasonDecl, ReasonDeclError};

    fn map_reasons() -> ReasonDecl<super::DeadLettering> {
        ReasonDecl::try_from_names(["transform_failed", "output_shape_unresolved"])
            .expect("two names")
    }

    #[test]
    fn a_declared_name_resolves() {
        let declared = map_reasons();
        assert_eq!(
            declared
                .resolve("transform_failed")
                .map(|r| r.name().to_owned()),
            Some("transform_failed".to_owned())
        );
    }

    #[test]
    fn an_undeclared_name_does_not_become_a_reason() {
        let declared = map_reasons();
        assert_eq!(
            declared.resolve("transform_faild"),
            None,
            "a typo became a new reason"
        );
        assert_eq!(declared.resolve(""), None);
        assert_eq!(
            declared.resolve("budget_exhausted"),
            None,
            "a diagnostic became a reason"
        );
    }

    #[test]
    fn an_empty_declaration_is_not_a_value() {
        assert_eq!(
            ReasonDecl::<super::DeadLettering>::try_from_names(Vec::<&str>::new()),
            Err(ReasonDeclError::Empty)
        );
    }

    #[test]
    fn a_duplicate_name_is_refused() {
        assert_eq!(
            ReasonDecl::<super::DeadLettering>::try_from_names(["a", "a"]),
            Err(ReasonDeclError::Duplicate("a".into()))
        );
    }

    #[test]
    fn the_two_vocabularies_are_distinct_types() {
        let dead: super::ReasonDecl<super::DeadLettering> =
            super::ReasonDecl::try_from_names(["shared"]).expect("one name");
        let suppress: super::ReasonDecl<super::Suppression> =
            super::ReasonDecl::try_from_names(["shared"]).expect("one name");

        let one = dead.resolve("shared").expect("declared");
        let other = suppress.resolve("shared").expect("declared");

        assert_eq!(one.name(), other.name());
        assert_eq!(one.name(), "shared");
    }

    #[test]
    fn the_declared_set_is_ordered() {
        let declared = map_reasons();
        assert_eq!(
            declared.names().collect::<Vec<_>>(),
            vec!["output_shape_unresolved", "transform_failed"]
        );
    }
}
