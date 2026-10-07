use circular_core::Value;
use std::path::Path;

const MIB: u64 = 1024 * 1024;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeArrivalLimits {
    pub(crate) arrivals_max_bytes: u64,
    pub(crate) arrivals_max_records: u64,
    pub(crate) total_max_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct JournalMeasure {
    pub(crate) bytes: u64,
    pub(crate) records: u64,
    pub(crate) file_bytes: u64,
    pub(crate) limits: RuntimeArrivalLimits,
}

impl JournalMeasure {
    pub(crate) fn of(
        journal: &engine::ProductDurableArrivalJournal,
        limits: RuntimeArrivalLimits,
    ) -> Result<Self, String> {
        let usage = journal.usage();
        Ok(Self {
            bytes: usage.bytes,
            records: usage.records,
            file_bytes: journal.file_bytes()?,
            limits,
        })
    }

    pub(crate) const fn exceeded(&self) -> Option<circular_actors::FailureDetail> {
        engine::activation_detail::journal::exceeded(
            self.bytes > self.limits.arrivals_max_bytes,
            self.records > self.limits.arrivals_max_records,
            self.file_bytes > self.limits.total_max_bytes,
        )
    }

    pub(crate) fn message(&self) -> String {
        format!(
            "arrival journal {} bytes (arrivals_max_bytes {}), {} records (arrivals_max_records {}), journal files {} bytes (total_max {}); input is still accepted and recorded, and the complete journal is retained",
            self.bytes,
            self.limits.arrivals_max_bytes,
            self.records,
            self.limits.arrivals_max_records,
            self.file_bytes,
            self.limits.total_max_bytes,
        )
    }

    pub(crate) fn value(&self) -> Result<Value, String> {
        Value::object([
            ("bytes", Value::UInt(self.bytes)),
            ("file_bytes", Value::UInt(self.file_bytes)),
            ("records", Value::UInt(self.records)),
            (
                "arrivals_max_bytes",
                Value::UInt(self.limits.arrivals_max_bytes),
            ),
            (
                "arrivals_max_records",
                Value::UInt(self.limits.arrivals_max_records),
            ),
            ("total_max_bytes", Value::UInt(self.limits.total_max_bytes)),
        ])
        .map_err(|error| format!("journal usage: {error:?}"))
    }
}

impl RuntimeArrivalLimits {
    pub(crate) fn from_mib(
        arrivals_max_mib: u64,
        arrivals_max_records: u64,
        total_max_mib: u64,
    ) -> Result<Self, String> {
        let arrivals_max_bytes = arrivals_max_mib.checked_mul(MIB).ok_or_else(|| {
            "runtime_arrivals.arrivals_max_mib exceeds the byte domain".to_owned()
        })?;
        let total_max_bytes = total_max_mib
            .checked_mul(MIB)
            .ok_or_else(|| "runtime_arrivals.total_max_mib exceeds the byte domain".to_owned())?;
        Ok(Self {
            arrivals_max_bytes,
            arrivals_max_records,
            total_max_bytes,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Inventory {
    orphan_arrivals: bool,
    missing_arrivals: bool,
}

pub(crate) fn startup_warnings(state_directory: &Path) -> Result<(), String> {
    let inventory = inventory(state_directory)?;
    if inventory.orphan_arrivals || inventory.missing_arrivals {
        eprintln!(
            "circular-daemon: runtime arrival inventory disagrees with the stream lifecycle — orphan arrivals {}, missing arrivals {}",
            inventory.orphan_arrivals, inventory.missing_arrivals
        );
    }
    Ok(())
}

fn inventory(state_directory: &Path) -> Result<Inventory, String> {
    let path = engine::state_journal::state_journal_path(state_directory);
    let bytes = match circular_store::SqliteJournal::namespace_payload_bytes(
        &path,
        engine::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
    ) {
        Ok(bytes) => bytes,
        Err(circular_store::SqliteJournalError::Missing(_)) => None,
        Err(error) => return Err(format!("cannot inventory runtime arrival journal: {error}")),
    };
    let known = engine::state_manifest::read_state_manifest(&path)?.is_some();
    Ok(Inventory {
        orphan_arrivals: bytes.is_some() && !known,
        missing_arrivals: bytes.is_none() && known,
    })
}
