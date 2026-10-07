
use std::collections::{BTreeMap, BTreeSet};

/// A normalized logical reference in the state-location namespace.
///
/// State and install references intentionally have no conversion between them.
/// The same normalized identity may occur once in each namespace without
/// making the values interchangeable.
///
/// ```compile_fail
/// use cli::{InstallLocationRef, StateLocationRef};
///
/// let state = StateLocationRef::from_normalized("primary");
/// let _: InstallLocationRef<_> = state;
/// ```
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StateLocationRef<I> {
    identity: I,
}

impl<I> StateLocationRef<I> {
    /// Preserves an identity already normalized by the owning Shell adapter.
    ///
    /// This constructor does not parse a path or consult process state.
    #[must_use]
    pub const fn from_normalized(identity: I) -> Self {
        Self { identity }
    }

    #[must_use]
    pub const fn identity(&self) -> &I {
        &self.identity
    }

    #[must_use]
    pub fn into_identity(self) -> I {
        self.identity
    }
}

/// A normalized logical reference in the install-location namespace.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InstallLocationRef<I> {
    identity: I,
}

impl<I> InstallLocationRef<I> {
    /// Preserves an identity already normalized by the owning Shell adapter.
    ///
    /// This constructor does not parse a path or consult process state.
    #[must_use]
    pub const fn from_normalized(identity: I) -> Self {
        Self { identity }
    }

    #[must_use]
    pub const fn identity(&self) -> &I {
        &self.identity
    }

    #[must_use]
    pub fn into_identity(self) -> I {
        self.identity
    }
}

/// One state-location catalog registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateLocationRegistration<I> {
    location: StateLocationRef<I>,
    marked_default: bool,
}

impl<I> StateLocationRegistration<I> {
    #[must_use]
    pub const fn new(location: StateLocationRef<I>) -> Self {
        Self {
            location,
            marked_default: false,
        }
    }

    #[must_use]
    pub const fn marked_default(location: StateLocationRef<I>) -> Self {
        Self {
            location,
            marked_default: true,
        }
    }

    #[must_use]
    pub const fn location(&self) -> &StateLocationRef<I> {
        &self.location
    }

    #[must_use]
    pub const fn is_marked_default(&self) -> bool {
        self.marked_default
    }
}

/// One install-location catalog registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallLocationRegistration<I> {
    location: InstallLocationRef<I>,
    marked_default: bool,
}

impl<I> InstallLocationRegistration<I> {
    #[must_use]
    pub const fn new(location: InstallLocationRef<I>) -> Self {
        Self {
            location,
            marked_default: false,
        }
    }

    #[must_use]
    pub const fn marked_default(location: InstallLocationRef<I>) -> Self {
        Self {
            location,
            marked_default: true,
        }
    }

    #[must_use]
    pub const fn location(&self) -> &InstallLocationRef<I> {
        &self.location
    }

    #[must_use]
    pub const fn is_marked_default(&self) -> bool {
        self.marked_default
    }
}

trait LogicalLocationRef: Clone + Ord {
    type Identity: Clone + Ord;

    fn identity(&self) -> &Self::Identity;
}

impl<I: Clone + Ord> LogicalLocationRef for StateLocationRef<I> {
    type Identity = I;

    fn identity(&self) -> &Self::Identity {
        self.identity()
    }
}

impl<I: Clone + Ord> LogicalLocationRef for InstallLocationRef<I> {
    type Identity = I;

    fn identity(&self) -> &Self::Identity {
        self.identity()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LocationCatalog<R>
where
    R: LogicalLocationRef,
{
    entries: BTreeMap<R::Identity, R>,
    marked_default: Option<R::Identity>,
}

impl<R> LocationCatalog<R>
where
    R: LogicalLocationRef,
{
    fn try_new(
        registrations: impl IntoIterator<Item = (R, bool)>,
    ) -> Result<Self, LocationCatalogError<R>> {
        let mut entries = BTreeMap::new();
        let mut defaults = BTreeSet::new();

        for (location, marked_default) in registrations {
            let identity = location.identity().clone();
            if entries.insert(identity, location.clone()).is_some() {
                return Err(LocationCatalogError::DuplicateRegistration { location });
            }
            if marked_default {
                defaults.insert(location);
            }
        }

        if defaults.len() > 1 {
            return Err(LocationCatalogError::MultipleDefaults {
                locations: defaults.into_iter().collect::<Vec<_>>().into_boxed_slice(),
            });
        }

        let marked_default = defaults
            .into_iter()
            .next()
            .map(|location| location.identity().clone());

        Ok(Self {
            entries,
            marked_default,
        })
    }

    fn get(&self, location: &R) -> Option<&R> {
        self.entries.get(location.identity())
    }

    fn marked_default(&self) -> Option<&R> {
        self.marked_default
            .as_ref()
            .and_then(|identity| self.entries.get(identity))
    }

    fn iter(&self) -> impl ExactSizeIterator<Item = &R> {
        self.entries.values()
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A catalog construction failure.
///
/// The fields of the public typed catalogs are private, so callers cannot
/// bypass duplicate/default validation by constructing a catalog directly.
///
/// ```compile_fail
/// use cli::StateLocationCatalog;
///
/// let _ = StateLocationCatalog::<u8> {
///     inner: todo!(),
/// };
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocationCatalogError<R> {
    DuplicateRegistration {
        location: R,
    },
    /// The locations are in normalized logical-identity order.
    MultipleDefaults {
        locations: Box<[R]>,
    },
}

/// A validated catalog in the state-location namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateLocationCatalog<I>
where
    I: Clone + Ord,
{
    inner: LocationCatalog<StateLocationRef<I>>,
}

impl<I> StateLocationCatalog<I>
where
    I: Clone + Ord,
{
    pub fn try_new(
        registrations: impl IntoIterator<Item = StateLocationRegistration<I>>,
    ) -> Result<Self, LocationCatalogError<StateLocationRef<I>>> {
        LocationCatalog::try_new(
            registrations
                .into_iter()
                .map(|registration| (registration.location, registration.marked_default)),
        )
        .map(|inner| Self { inner })
    }

    #[must_use]
    pub fn get(&self, location: &StateLocationRef<I>) -> Option<&StateLocationRef<I>> {
        self.inner.get(location)
    }

    #[must_use]
    pub fn marked_default(&self) -> Option<&StateLocationRef<I>> {
        self.inner.marked_default()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &StateLocationRef<I>> {
        self.inner.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// A validated catalog in the install-location namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallLocationCatalog<I>
where
    I: Clone + Ord,
{
    inner: LocationCatalog<InstallLocationRef<I>>,
}

impl<I> InstallLocationCatalog<I>
where
    I: Clone + Ord,
{
    pub fn try_new(
        registrations: impl IntoIterator<Item = InstallLocationRegistration<I>>,
    ) -> Result<Self, LocationCatalogError<InstallLocationRef<I>>> {
        LocationCatalog::try_new(
            registrations
                .into_iter()
                .map(|registration| (registration.location, registration.marked_default)),
        )
        .map(|inner| Self { inner })
    }

    #[must_use]
    pub fn get(&self, location: &InstallLocationRef<I>) -> Option<&InstallLocationRef<I>> {
        self.inner.get(location)
    }

    #[must_use]
    pub fn marked_default(&self) -> Option<&InstallLocationRef<I>> {
        self.inner.marked_default()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &InstallLocationRef<I>> {
        self.inner.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// A deterministic logical-location selection failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocationSelectionError<R> {
    /// An explicit reference is resolved by itself and never falls back.
    UnknownExplicit { requested: R },
    /// The candidates are in normalized logical-identity order.
    Ambiguous { candidates: Box<[R]> },
}

pub type StateLocationSelectionError<I> = LocationSelectionError<StateLocationRef<I>>;
pub type InstallLocationSelectionError<I> = LocationSelectionError<InstallLocationRef<I>>;

fn select_location<R, F>(
    explicit: Option<&R>,
    catalog: &LocationCatalog<R>,
    conventional: F,
) -> Result<R, LocationSelectionError<R>>
where
    R: LogicalLocationRef,
    F: FnOnce() -> R,
{
    if let Some(explicit) = explicit {
        return catalog.get(explicit).cloned().ok_or_else(|| {
            LocationSelectionError::UnknownExplicit {
                requested: explicit.clone(),
            }
        });
    }

    if let Some(marked_default) = catalog.marked_default() {
        return Ok(marked_default.clone());
    }

    match catalog.len() {
        0 => Ok(conventional()),
        1 => Ok(catalog
            .iter()
            .next()
            .expect("a singleton catalog has one location")
            .clone()),
        _ => Err(LocationSelectionError::Ambiguous {
            candidates: catalog
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }),
    }
}

/// Selects a state location from its explicit argument and typed catalog.
///
/// Only an empty catalog calls `conventional`; that provider's logical value
/// and process-state independence belong to the later physical-location owner.
///
/// ```compile_fail
/// use cli::{
///     InstallLocationCatalog, StateLocationRef, select_state,
/// };
///
/// let installs = InstallLocationCatalog::<&str>::try_new([]).unwrap();
/// let _ = select_state(None, &installs, || {
///     StateLocationRef::from_normalized("state")
/// });
/// ```
pub fn select_state<I, F>(
    explicit: Option<&StateLocationRef<I>>,
    catalog: &StateLocationCatalog<I>,
    conventional: F,
) -> Result<StateLocationRef<I>, StateLocationSelectionError<I>>
where
    I: Clone + Ord,
    F: FnOnce() -> StateLocationRef<I>,
{
    select_location(explicit, &catalog.inner, conventional)
}

/// Selects an install location using only its explicit argument and typed catalog.
pub fn select_install<I, F>(
    explicit: Option<&InstallLocationRef<I>>,
    catalog: &InstallLocationCatalog<I>,
    conventional: F,
) -> Result<InstallLocationRef<I>, InstallLocationSelectionError<I>>
where
    I: Clone + Ord,
    F: FnOnce() -> InstallLocationRef<I>,
{
    select_location(explicit, &catalog.inner, conventional)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn state(identity: u8) -> StateLocationRef<u8> {
        StateLocationRef::from_normalized(identity)
    }

    fn install(identity: u8) -> InstallLocationRef<u8> {
        InstallLocationRef::from_normalized(identity)
    }

    fn state_entry(identity: u8) -> StateLocationRegistration<u8> {
        StateLocationRegistration::new(state(identity))
    }

    fn state_default(identity: u8) -> StateLocationRegistration<u8> {
        StateLocationRegistration::marked_default(state(identity))
    }

    fn install_entry(identity: u8) -> InstallLocationRegistration<u8> {
        InstallLocationRegistration::new(install(identity))
    }

    fn install_default(identity: u8) -> InstallLocationRegistration<u8> {
        InstallLocationRegistration::marked_default(install(identity))
    }

    #[test]
    fn equal_identity_text_remains_in_two_disjoint_location_namespaces() {
        let state = StateLocationRef::from_normalized("same");
        let install = InstallLocationRef::from_normalized("same");

        assert_eq!(state.identity(), install.identity());
        assert_eq!(state.into_identity(), "same");
        assert_eq!(install.into_identity(), "same");
    }

    #[test]
    fn duplicate_registration_and_multiple_defaults_are_rejected() {
        assert_eq!(
            StateLocationCatalog::try_new([state_entry(3), state_default(3)]),
            Err(LocationCatalogError::DuplicateRegistration { location: state(3) })
        );

        assert_eq!(
            InstallLocationCatalog::try_new([
                install_default(9),
                install_default(2),
                install_entry(5),
            ]),
            Err(LocationCatalogError::MultipleDefaults {
                locations: Box::from([install(2), install(9)]),
            })
        );
    }

    #[test]
    fn omitted_selection_exhausts_empty_singleton_default_and_ambiguous_cases() {
        let conventional_calls = Cell::new(0);
        let empty = StateLocationCatalog::try_new([]).expect("empty catalog is valid");
        assert_eq!(
            select_state(None, &empty, || {
                conventional_calls.set(conventional_calls.get() + 1);
                state(90)
            }),
            Ok(state(90))
        );
        assert_eq!(conventional_calls.get(), 1);

        let singleton = StateLocationCatalog::try_new([state_entry(7)]).expect("singleton catalog");
        assert_eq!(select_state(None, &singleton, || state(90)), Ok(state(7)));

        let marked =
            StateLocationCatalog::try_new([state_entry(7), state_default(4), state_entry(1)])
                .expect("one marked default");
        assert_eq!(select_state(None, &marked, || state(90)), Ok(state(4)));

        let ambiguous =
            StateLocationCatalog::try_new([state_entry(8), state_entry(2), state_entry(5)])
                .expect("unmarked catalog");
        assert_eq!(
            select_state(None, &ambiguous, || state(90)),
            Err(LocationSelectionError::Ambiguous {
                candidates: Box::from([state(2), state(5), state(8)]),
            })
        );

        let empty = InstallLocationCatalog::try_new([]).expect("empty catalog is valid");
        assert_eq!(
            select_install(None, &empty, || install(90)),
            Ok(install(90))
        );

        let singleton =
            InstallLocationCatalog::try_new([install_entry(7)]).expect("singleton catalog");
        assert_eq!(
            select_install(None, &singleton, || install(90)),
            Ok(install(7))
        );

        let marked = InstallLocationCatalog::try_new([
            install_entry(7),
            install_default(4),
            install_entry(1),
        ])
        .expect("one marked default");
        assert_eq!(
            select_install(None, &marked, || install(90)),
            Ok(install(4))
        );

        let ambiguous =
            InstallLocationCatalog::try_new([install_entry(8), install_entry(2), install_entry(5)])
                .expect("unmarked catalog");
        assert_eq!(
            select_install(None, &ambiguous, || install(90)),
            Err(LocationSelectionError::Ambiguous {
                candidates: Box::from([install(2), install(5), install(8)]),
            })
        );
    }

    #[test]
    fn explicit_selection_is_exclusive_and_never_calls_a_fallback() {
        let conventional_calls = Cell::new(0);
        let catalog =
            StateLocationCatalog::try_new([state_entry(1), state_default(2), state_entry(3)])
                .expect("catalog");

        assert_eq!(
            select_state(Some(&state(3)), &catalog, || {
                conventional_calls.set(conventional_calls.get() + 1);
                state(90)
            }),
            Ok(state(3))
        );
        assert_eq!(conventional_calls.get(), 0);

        assert_eq!(
            select_state(Some(&state(99)), &catalog, || {
                conventional_calls.set(conventional_calls.get() + 1);
                state(90)
            }),
            Err(LocationSelectionError::UnknownExplicit {
                requested: state(99),
            })
        );
        assert_eq!(conventional_calls.get(), 0);
    }

    fn permutations<T: Clone>(values: &[T]) -> Vec<Vec<T>> {
        fn visit<T: Clone>(remaining: Vec<T>, prefix: Vec<T>, output: &mut Vec<Vec<T>>) {
            if remaining.is_empty() {
                output.push(prefix);
                return;
            }
            for index in 0..remaining.len() {
                let mut next_remaining = remaining.clone();
                let value = next_remaining.remove(index);
                let mut next_prefix = prefix.clone();
                next_prefix.push(value);
                visit(next_remaining, next_prefix, output);
            }
        }

        let mut output = Vec::new();
        visit(values.to_vec(), Vec::new(), &mut output);
        output
    }

    #[test]
    fn insertion_permutation_does_not_change_selection_or_ambiguity_order() {
        let registrations = [state_entry(8), state_default(3), state_entry(5)];
        for permutation in permutations(&registrations) {
            let catalog = StateLocationCatalog::try_new(permutation).expect("valid permutation");
            assert_eq!(select_state(None, &catalog, || state(90)), Ok(state(3)));
            assert_eq!(
                catalog.iter().cloned().collect::<Vec<_>>(),
                vec![state(3), state(5), state(8)]
            );
        }

        let registrations = [state_entry(8), state_entry(3), state_entry(5)];
        for permutation in permutations(&registrations) {
            let catalog = StateLocationCatalog::try_new(permutation).expect("valid permutation");
            assert_eq!(
                select_state(None, &catalog, || state(90)),
                Err(LocationSelectionError::Ambiguous {
                    candidates: Box::from([state(3), state(5), state(8)]),
                })
            );
        }
    }

    #[test]
    fn explicit_selection_is_stable_under_catalog_extension_and_default_changes() {
        let explicit = state(4);
        let catalogs = [
            StateLocationCatalog::try_new([state_entry(4)]).expect("base"),
            StateLocationCatalog::try_new([state_entry(4), state_default(7)])
                .expect("extended with default"),
            StateLocationCatalog::try_new([
                state_entry(9),
                state_entry(4),
                state_default(1),
                state_entry(6),
            ])
            .expect("reordered extension"),
        ];

        for catalog in &catalogs {
            assert_eq!(
                select_state(Some(&explicit), catalog, || state(90)),
                Ok(explicit.clone())
            );
        }
    }

    #[test]
    fn state_and_install_catalog_changes_are_independent() {
        let state_catalogs = [
            (
                StateLocationCatalog::try_new([state_entry(1)]).expect("state singleton"),
                state(1),
            ),
            (
                StateLocationCatalog::try_new([state_entry(1), state_default(2)])
                    .expect("state default"),
                state(2),
            ),
        ];
        let install_catalogs = [
            (
                InstallLocationCatalog::try_new([install_entry(7)]).expect("install singleton"),
                install(7),
            ),
            (
                InstallLocationCatalog::try_new([install_entry(7), install_default(8)])
                    .expect("install default"),
                install(8),
            ),
        ];

        for (state_catalog, expected_state) in &state_catalogs {
            for (install_catalog, expected_install) in &install_catalogs {
                assert_eq!(
                    select_state(None, state_catalog, || state(90)),
                    Ok(expected_state.clone())
                );
                assert_eq!(
                    select_install(None, install_catalog, || install(90)),
                    Ok(expected_install.clone())
                );
            }
        }
    }
}
