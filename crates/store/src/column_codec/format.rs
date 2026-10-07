//! Storage vocabulary. All
//! physical discriminants live here, independently of the logical
//! record/transaction codecs. Zero and unlisted discriminants reject.
//! Observation and display names are not a physical table: the stored byte is
//! the `BuiltinObservationName` catalog tag itself.
//! Unpublished: the version stays 1 and a format change edits it in place.
pub const VERSION: u8 = 1;
pub const NORMAL: u8 = 1;
pub const KEYFRAME: u8 = 2;
pub const CURRENT: u8 = 1;
pub const COMMITTED: u8 = 2;
pub const LOCAL: u8 = 3;
pub const ACTOR: u8 = 1;
pub const EDGE: u8 = 2;
pub const EFFECT_DELIVERY: u8 = 1;
pub const EFFECT_POLL: u8 = 2;
pub const SCOPE: u8 = 4;
pub const PORT: u8 = 5;
pub const ABSOLUTE: u8 = 1;
pub const INCREASE: u8 = 2;
pub const DECREASE: u8 = 3;
pub const STAMP_ABSOLUTE: u8 = 1;
pub const STAMP_DELTA: u8 = 2;
pub const STAMP_LOCAL: u8 = 3;
pub const STAMP_PREVIOUS: u8 = 4;
pub const NEXT: u8 = 1;
pub const EXPLICIT: u8 = 2;
pub const ABSENT: u8 = 1;
pub const PRESENT: u8 = 2;
pub const BOUNDARY: u8 = 1;
pub const STRUCTURE: u8 = 2;
pub const DISPLAY: u8 = 4;
pub const OBSERVATION: u8 = 5;
pub const ARRIVAL: u8 = 1;
pub const RESERVATION: u8 = 2;
pub const ADMISSION: u8 = 3;
pub const EMISSION: u8 = 4;
pub const MANIFEST: u8 = 1;
pub const REVISION: u8 = 2;
pub const DISPLAY_ITEM: u8 = 1;
pub const LIFECYCLE: u8 = 1;
pub const DIAGNOSTIC: u8 = 2;
pub const ACCOUNTING: u8 = 3;
pub const DEAD_LETTER: u8 = 4;
pub const REPLAY: u8 = 5;
pub const RESTART: u8 = 6;
pub const CHECKPOINT_FACT: u8 = 7;
pub const OBS_STREAM: u8 = 1;
pub const OBS_GLOBAL: u8 = 2;
pub const OBS_CHECKPOINT: u8 = 3;
pub const FROM_EDGE: u8 = 1;
pub const FROM_TIMER: u8 = 2;
pub const FROM_EFFECT: u8 = 3;
pub const FROM_EXTERNAL: u8 = 4;
pub const OWNED: u8 = 1;
pub const EFFECT_BODY: u8 = 2;
pub const EMITTED: u8 = 4;
pub const BASE: u64 = 1;
pub const ATTRIBUTED: u64 = 2;
pub const INDEX: u64 = 4;
pub const OBSERVED: u64 = 8;
pub const BUCKET: u64 = 16;
pub const OK: u64 = 32;
pub const KNOWN_FLAGS: u64 = BASE | ATTRIBUTED | INDEX | OBSERVED | BUCKET | OK;
pub const APPEND: u8 = 1;
pub const APPEND_OBSERVATION: u8 = 2;
pub const OPEN_OUTBOX: u8 = 5;
pub const SUBMIT_OUTBOX: u8 = 6;
pub const ACQUIRE_OUTBOX: u8 = 7;
pub const SETTLE_OUTBOX: u8 = 8;
pub const CANCEL_OUTBOX: u8 = 9;
pub const OPEN_APPROVAL: u8 = 10;
pub const APPROVE_APPROVAL: u8 = 11;
pub const SETTLE_APPROVAL: u8 = 12;
pub const REPLACE_CHECKPOINT: u8 = 13;
pub const SETTLE_CHECKPOINT: u8 = 14;
pub const OPERATIONS: [u8; 12] = [
    APPEND,
    APPEND_OBSERVATION,
    OPEN_OUTBOX,
    SUBMIT_OUTBOX,
    ACQUIRE_OUTBOX,
    SETTLE_OUTBOX,
    CANCEL_OUTBOX,
    OPEN_APPROVAL,
    APPROVE_APPROVAL,
    SETTLE_APPROVAL,
    REPLACE_CHECKPOINT,
    SETTLE_CHECKPOINT,
];
pub const STAMP_TIE_ORDER: [u8; 4] = [STAMP_LOCAL, STAMP_PREVIOUS, STAMP_ABSOLUTE, STAMP_DELTA];
pub const NUMBER_TIE_ORDER: [u8; 3] = [ABSOLUTE, INCREASE, DECREASE];
