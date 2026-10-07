
use crate::memory::{AppendBatch, AppendResult};
use crate::query::{Page, Query, StoreError};
use crate::record::StoreSchema;

pub trait Store<S: StoreSchema> {
    fn append(&mut self, batch: AppendBatch<S>) -> AppendResult<S>;
    fn query(&self, query: &Query<S>) -> Result<Page<S>, StoreError<S>>;
}
