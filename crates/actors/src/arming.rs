
use circular_runtime::ScheduleCorrelation;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArmingCorrelation(u64);

impl ArmingCorrelation {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn correlation(self) -> ScheduleCorrelation {
        ScheduleCorrelation::new(self.0)
    }
}
