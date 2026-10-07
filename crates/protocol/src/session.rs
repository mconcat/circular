
use crate::Partition;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TransportTrust {
    Remote,
    LocalUser,
    LocalOwner,
}

impl TransportTrust {
    #[must_use]
    pub const fn allows(self, minimum: Self) -> bool {
        self as u8 >= minimum as u8
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SessionRole<S> {
    Reader,
    Writer { scope: S },
    Operator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MissingReader;

impl fmt::Display for MissingReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("session role set must include Reader")
    }
}

impl std::error::Error for MissingReader {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRoles<S>(BTreeSet<SessionRole<S>>);

impl<S: Ord> SessionRoles<S> {
    #[must_use]
    pub fn reader_only() -> Self {
        Self(BTreeSet::from([SessionRole::Reader]))
    }

    pub fn try_from_roles(
        roles: impl IntoIterator<Item = SessionRole<S>>,
    ) -> Result<Self, MissingReader> {
        let roles = roles.into_iter().collect::<BTreeSet<_>>();
        if roles.contains(&SessionRole::Reader) {
            Ok(Self(roles))
        } else {
            Err(MissingReader)
        }
    }

    #[must_use]
    pub fn with_role(mut self, role: SessionRole<S>) -> Self {
        self.0.insert(role);
        self
    }

    #[must_use]
    pub fn contains(&self, role: &SessionRole<S>) -> bool {
        self.0.contains(role)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SessionRole<S>> {
        self.0.iter()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DuplicateFeature {
    partition: Partition,
}

impl DuplicateFeature {
    #[must_use]
    pub const fn partition(self) -> Partition {
        self.partition
    }
}

impl fmt::Display for DuplicateFeature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "duplicate feature minor for partition {:?}",
            self.partition
        )
    }
}

impl std::error::Error for DuplicateFeature {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureSet<M>(BTreeMap<Partition, M>);

impl<M> FeatureSet<M> {
    pub fn try_new(
        entries: impl IntoIterator<Item = (Partition, M)>,
    ) -> Result<Self, DuplicateFeature> {
        let mut features = BTreeMap::new();
        for (partition, minor) in entries {
            if features.insert(partition, minor).is_some() {
                return Err(DuplicateFeature { partition });
            }
        }
        Ok(Self(features))
    }

    #[must_use]
    pub fn get(&self, partition: Partition) -> Option<&M> {
        self.0.get(&partition)
    }

    pub fn iter(&self) -> impl Iterator<Item = (Partition, &M)> {
        self.0.iter().map(|(partition, minor)| (*partition, minor))
    }
}

impl<M: Clone + Ord> FeatureSet<M> {
    #[must_use]
    pub fn negotiate(&self, other: &Self) -> Self {
        let mut agreed = BTreeMap::new();
        for (partition, local) in &self.0 {
            if let Some(remote) = other.0.get(partition) {
                agreed.insert(*partition, std::cmp::min(local, remote).clone());
            }
        }
        Self(agreed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hello<V, M, S> {
    protocol_version: V,
    features: FeatureSet<M>,
    requested_roles: SessionRoles<S>,
}

impl<V, M, S> Hello<V, M, S> {
    #[must_use]
    pub const fn new(
        protocol_version: V,
        features: FeatureSet<M>,
        requested_roles: SessionRoles<S>,
    ) -> Self {
        Self {
            protocol_version,
            features,
            requested_roles,
        }
    }

    #[must_use]
    pub const fn protocol_version(&self) -> &V {
        &self.protocol_version
    }

    #[must_use]
    pub const fn features(&self) -> &FeatureSet<M> {
        &self.features
    }

    #[must_use]
    pub const fn requested_roles(&self) -> &SessionRoles<S> {
        &self.requested_roles
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EstablishedSession<V, M, S, T> {
    protocol_version: V,
    features: FeatureSet<M>,
    roles: SessionRoles<S>,
    trust: TransportTrust,
    token: T,
}

mod private {
    pub trait SealedEstablishedSession {}
}

pub trait EstablishedSessionValue: private::SealedEstablishedSession {}

impl<V, M, S, T> private::SealedEstablishedSession for EstablishedSession<V, M, S, T> {}
impl<V, M, S, T> EstablishedSessionValue for EstablishedSession<V, M, S, T> {}

impl<V, M, S, T> EstablishedSession<V, M, S, T> {
    #[must_use]
    pub(crate) const fn new(
        protocol_version: V,
        features: FeatureSet<M>,
        roles: SessionRoles<S>,
        trust: TransportTrust,
        token: T,
    ) -> Self {
        Self {
            protocol_version,
            features,
            roles,
            trust,
            token,
        }
    }

    #[must_use]
    pub const fn protocol_version(&self) -> &V {
        &self.protocol_version
    }

    #[must_use]
    pub const fn features(&self) -> &FeatureSet<M> {
        &self.features
    }

    #[must_use]
    pub const fn roles(&self) -> &SessionRoles<S> {
        &self.roles
    }

    #[must_use]
    pub const fn trust(&self) -> TransportTrust {
        self.trust
    }

    #[must_use]
    pub const fn token(&self) -> &T {
        &self.token
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HelloAck<E: EstablishedSessionValue, R> {
    Established(E),
    Rejected(R),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionState<E: EstablishedSessionValue> {
    AwaitingHello,
    Established(E),
    Closed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_cannot_be_built_without_reader() {
        let result = SessionRoles::<u8>::try_from_roles([SessionRole::Operator]);
        assert_eq!(result, Err(MissingReader));
    }

    #[test]
    fn negotiated_features_are_the_partitionwise_meet() {
        let left = FeatureSet::try_new([(Partition::Query, 4_u8), (Partition::Subscription, 2)])
            .expect("unique partition");
        let right = FeatureSet::try_new([(Partition::Query, 3_u8), (Partition::Declaration, 7)])
            .expect("unique partition");

        let agreed = left.negotiate(&right);
        assert_eq!(agreed.get(Partition::Query), Some(&3));
        assert_eq!(agreed.get(Partition::Subscription), None);
        assert_eq!(agreed.get(Partition::Declaration), None);
    }

    #[test]
    fn trust_is_monotone() {
        assert!(TransportTrust::LocalOwner.allows(TransportTrust::Remote));
        assert!(!TransportTrust::Remote.allows(TransportTrust::LocalUser));
    }
}
