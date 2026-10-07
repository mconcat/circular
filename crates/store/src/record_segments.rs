use crate::record::EncodedRow;
use crate::{Record, StoreSchema};
use circular_core::{EncodedPayload, RevisionEpochId};
use std::{
    borrow::Cow,
    ops::{Bound, RangeBounds},
    sync::Arc,
};

#[derive(Debug)]
enum Rows<S: StoreSchema> {
    Decoded(Box<[Record<S>]>),
    Encoded {
        rows: Box<[EncodedPayload]>,
        revisions: Box<[u64]>,
    },
}

impl<S: StoreSchema> Clone for Rows<S> {
    fn clone(&self) -> Self {
        match self {
            Self::Decoded(rows) => Self::Decoded(rows.clone()),
            Self::Encoded { rows, revisions } => Self::Encoded {
                rows: rows.clone(),
                revisions: revisions.clone(),
            },
        }
    }
}

impl<S: StoreSchema> Rows<S> {
    fn new(rows: Vec<EncodedRow<S>>) -> Self {
        let codec = S::row_codec();
        let mut records = Vec::with_capacity(rows.len());
        let mut encoded = Vec::with_capacity(rows.len());
        let mut revisions = Vec::with_capacity(rows.len());
        let mut as_bytes = codec.is_some();
        for row in rows {
            let (record, bytes) = row.into_parts();
            if let Some(codec) = codec.filter(|_| as_bytes) {
                revisions.push(
                    record
                        .header()
                        .position()
                        .stamp()
                        .map_or(0, |stamp| stamp.revision().get()),
                );
                match bytes {
                    Some(bytes) => encoded.push(bytes),
                    None => match codec.encode(&record) {
                        Some(payload) if codec.decode(&payload).as_ref() == Some(&record) => {
                            encoded.push(payload);
                        }
                        _ => as_bytes = false,
                    },
                }
            }
            records.push(record);
        }
        if as_bytes {
            Self::Encoded {
                rows: encoded.into_boxed_slice(),
                revisions: revisions.into_boxed_slice(),
            }
        } else {
            Self::Decoded(records.into_boxed_slice())
        }
    }

    fn replace(&mut self, index: usize, record: Record<S>) {
        match self {
            Self::Decoded(rows) => rows[index] = record,
            Self::Encoded { rows, revisions } => {
                if let Some(bytes) = S::row_codec().and_then(|codec| {
                    let bytes = codec.encode(&record)?;
                    (codec.decode(&bytes).as_ref() == Some(&record)).then_some(bytes)
                }) {
                    revisions[index] = record
                        .header()
                        .position()
                        .stamp()
                        .map_or(0, |stamp| stamp.revision().get());
                    rows[index] = bytes;
                } else {
                    let mut decoded = (0..rows.len())
                        .map(|i| self.at(i).into_owned())
                        .collect::<Vec<_>>();
                    decoded[index] = record;
                    *self = Self::Decoded(decoded.into_boxed_slice());
                }
            }
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Decoded(rows) => rows.len(),
            Self::Encoded { rows, .. } => rows.len(),
        }
    }

    fn revision(&self, index: usize) -> Option<RevisionEpochId> {
        match self {
            Self::Decoded(rows) => rows
                .get(index)?
                .header()
                .position()
                .stamp()
                .map(circular_core::Stamp::revision),
            Self::Encoded { revisions, .. } => RevisionEpochId::new(*revisions.get(index)?),
        }
    }

    fn decoded(&self, index: usize) -> Option<Record<S>> {
        let Self::Encoded { rows, .. } = self else {
            return None;
        };
        S::row_codec()?.decode(rows.get(index)?)
    }

    fn address(&self, index: usize) -> *const u8 {
        match self {
            Self::Decoded(rows) => std::ptr::from_ref(&rows[index]).cast::<u8>(),
            Self::Encoded { rows, .. } => rows[index].as_bytes().as_ptr(),
        }
    }

    fn stored(&self, index: usize) -> StoredRow<'_, S> {
        match self {
            Self::Decoded(rows) => StoredRow::Decoded(&rows[index]),
            Self::Encoded { rows, .. } => StoredRow::Encoded(&rows[index]),
        }
    }

    fn at(&self, index: usize) -> Cow<'_, Record<S>> {
        #[cfg(feature = "test-support")]
        crate::read_measurement::record_row();
        match self {
            Self::Decoded(rows) => Cow::Borrowed(&rows[index]),
            Self::Encoded { .. } => Cow::Owned(
                self.decoded(index)
                    .expect("a sequence that read back when written also reads back when read"),
            ),
        }
    }
}

#[derive(Debug)]
enum Segment<S: StoreSchema> {
    Leaf(Rows<S>, Box<[u64]>),
    Branch {
        left: Arc<Self>,
        right: Arc<Self>,
        len: usize,
        height: usize,
    },
}
impl<S: StoreSchema> Segment<S> {
    fn len(&self) -> usize {
        match self {
            Self::Leaf(rows, _) => rows.len(),
            Self::Branch { len, .. } => *len,
        }
    }
    fn height(&self) -> usize {
        match self {
            Self::Leaf(..) => 1,
            Self::Branch { height, .. } => *height,
        }
    }
    fn at(&self, index: usize) -> Cow<'_, Record<S>> {
        match self {
            Self::Leaf(rows, _) => rows.at(index),
            Self::Branch { left, right, .. } => {
                if index < left.len() {
                    left.at(index)
                } else {
                    right.at(index - left.len())
                }
            }
        }
    }
    fn replace(node: &mut Arc<Self>, index: usize, record: Record<S>) {
        match Arc::make_mut(node) {
            Self::Leaf(rows, _) => rows.replace(index, record),
            Self::Branch { left, right, .. } => {
                if index < left.len() {
                    Self::replace(left, index, record);
                } else {
                    Self::replace(right, index - left.len(), record);
                }
            }
        }
    }
    fn leaf_at(&self, index: usize, base: usize) -> (usize, &Rows<S>) {
        let (base, rows, _) = self.committed_leaf_at(index, base);
        (base, rows)
    }
    fn committed_leaf_at(&self, index: usize, base: usize) -> (usize, &Rows<S>, &[u64]) {
        match self {
            Self::Leaf(rows, commits) => (base, rows, commits),
            Self::Branch { left, right, .. } => {
                if index < left.len() {
                    left.committed_leaf_at(index, base)
                } else {
                    right.committed_leaf_at(index - left.len(), base + left.len())
                }
            }
        }
    }
    fn branch(left: Arc<Self>, right: Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Branch {
            len: left.len() + right.len(),
            height: 1 + left.height().max(right.height()),
            left,
            right,
        })
    }
    fn append(left: Arc<Self>, right: Arc<Self>) -> Arc<Self> {
        if left.height() > right.height() + 1 {
            let Self::Branch {
                left: a, right: b, ..
            } = &*left
            else {
                unreachable!()
            };
            let tail = Self::append(b.clone(), right);
            if tail.height() > a.height() + 1 {
                let Self::Branch {
                    left: c, right: d, ..
                } = &*tail
                else {
                    unreachable!()
                };
                return Self::branch(Self::branch(a.clone(), c.clone()), d.clone());
            }
            return Self::branch(a.clone(), tail);
        }
        Self::branch(left, right)
    }
}
impl<S: StoreSchema> Clone for Segment<S> {
    fn clone(&self) -> Self {
        match self {
            Self::Leaf(rows, commits) => Self::Leaf(rows.clone(), commits.clone()),
            Self::Branch {
                left,
                right,
                len,
                height,
            } => Self::Branch {
                left: left.clone(),
                right: right.clone(),
                len: *len,
                height: *height,
            },
        }
    }
}
#[derive(Debug)]
pub struct RecordSegments<S: StoreSchema> {
    root: Option<Arc<Segment<S>>>,
}
impl<S: StoreSchema> Default for RecordSegments<S> {
    fn default() -> Self {
        Self { root: None }
    }
}
impl<S: StoreSchema> Clone for RecordSegments<S> {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
        }
    }
}
impl<S: StoreSchema> RecordSegments<S> {
    pub fn len(&self) -> usize {
        self.root.as_ref().map_or(0, |r| r.len())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn iter(&self) -> RecordIter<'_, S> {
        self.slice(..).iter()
    }
    pub fn rows(&self) -> RowIter<'_, S> {
        self.slice(..).rows()
    }
    pub fn row(&self, index: usize) -> Cow<'_, Record<S>> {
        self.root
            .as_ref()
            .expect("nonempty record prefix")
            .at(index)
    }
    pub fn get(&self, index: usize) -> Option<Cow<'_, Record<S>>> {
        (index < self.len()).then(|| self.row(index))
    }
    #[must_use]
    pub fn record_row(&self, index: usize) -> RecordRow<'_, S> {
        let (base, rows, commits) = self
            .root
            .as_ref()
            .expect("nonempty record prefix")
            .committed_leaf_at(index, 0);
        assert!(index - base < rows.len(), "record position out of prefix");
        RecordRow {
            rows,
            offset: index - base,
            position: index,
            commit: commits.get(index - base).copied().unwrap_or(0),
        }
    }
    #[must_use]
    pub fn row_commit(&self, index: usize) -> u64 {
        let (base, _, commits) = self
            .root
            .as_ref()
            .expect("nonempty record prefix")
            .committed_leaf_at(index, 0);
        commits.get(index - base).copied().unwrap_or(0)
    }
    #[must_use]
    pub fn row_revision(&self, index: usize) -> Option<RevisionEpochId> {
        let (base, rows) = self
            .root
            .as_ref()
            .expect("nonempty record prefix")
            .leaf_at(index, 0);
        rows.revision(index - base)
    }
    #[must_use]
    pub fn row_address(&self, index: usize) -> *const u8 {
        let (base, rows) = self
            .root
            .as_ref()
            .expect("nonempty record prefix")
            .leaf_at(index, 0);
        rows.address(index - base)
    }
    pub fn last(&self) -> Option<Cow<'_, Record<S>>> {
        self.len().checked_sub(1).map(|i| self.row(i))
    }
    pub fn to_vec(&self) -> Vec<Record<S>> {
        self.iter().map(Cow::into_owned).collect()
    }
    pub fn slice(&self, range: impl RangeBounds<usize>) -> RecordSlice<'_, S> {
        let start = match range.start_bound() {
            Bound::Included(i) => *i,
            Bound::Excluded(i) => i + 1,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(i) => i + 1,
            Bound::Excluded(i) => *i,
            Bound::Unbounded => self.len(),
        };
        assert!(start <= end && end <= self.len());
        RecordSlice {
            records: self,
            start,
            end,
        }
    }
    pub(crate) fn append_committed(&mut self, rows: Vec<EncodedRow<S>>, commits: Vec<u64>) {
        if rows.is_empty() {
            return;
        }
        debug_assert!(commits.is_empty() || commits.len() == rows.len());
        let next = Arc::new(Segment::Leaf(Rows::new(rows), commits.into_boxed_slice()));
        self.root = Some(match self.root.take() {
            Some(root) => Segment::append(root, next),
            None => next,
        });
    }
    pub(crate) fn push_committed(&mut self, row: Record<S>, commit: u64) {
        self.append_committed(vec![EncodedRow::new(row)], vec![commit]);
    }
    pub fn replace(&mut self, index: usize, record: Record<S>) {
        assert!(index < self.len());
        Segment::replace(self.root.as_mut().expect("nonempty records"), index, record);
    }
}
#[derive(Debug)]
pub struct RecordSlice<'a, S: StoreSchema> {
    records: &'a RecordSegments<S>,
    start: usize,
    end: usize,
}
impl<S: StoreSchema> Copy for RecordSlice<'_, S> {}
impl<S: StoreSchema> Clone for RecordSlice<'_, S> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'a, S: StoreSchema> RecordSlice<'a, S> {
    pub fn len(&self) -> usize {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
    pub fn iter(&self) -> RecordIter<'a, S> {
        RecordIter {
            records: self.records,
            start: self.start,
            end: self.end,
            front: None,
            back: None,
        }
    }
    pub fn row(&self, index: usize) -> Cow<'a, Record<S>> {
        assert!(index < self.len());
        self.records.row(self.start + index)
    }
    pub fn rows(&self) -> RowIter<'a, S> {
        RowIter {
            records: self.records,
            start: self.start,
            end: self.end,
            leaf: None,
        }
    }
    pub fn get(&self, index: usize) -> Option<Cow<'a, Record<S>>> {
        (index < self.len()).then(|| self.records.row(self.start + index))
    }
    pub fn to_vec(&self) -> Vec<Record<S>> {
        self.iter().map(Cow::into_owned).collect()
    }
}
impl<S: StoreSchema, const N: usize> PartialEq<&[Record<S>; N]> for RecordSlice<'_, S> {
    fn eq(&self, rhs: &&[Record<S>; N]) -> bool {
        self.len() == N
            && self
                .iter()
                .zip(rhs.iter())
                .all(|(own, other)| &*own == other)
    }
}
impl<S: StoreSchema> PartialEq<&[Record<S>]> for RecordSlice<'_, S> {
    fn eq(&self, rhs: &&[Record<S>]) -> bool {
        self.len() == rhs.len()
            && self
                .iter()
                .zip(rhs.iter())
                .all(|(own, other)| &*own == other)
    }
}
pub struct RecordIter<'a, S: StoreSchema> {
    records: &'a RecordSegments<S>,
    start: usize,
    end: usize,
    front: Option<(usize, &'a Rows<S>)>,
    back: Option<(usize, &'a Rows<S>)>,
}
impl<'a, S: StoreSchema> RecordIter<'a, S> {
    fn at(
        records: &'a RecordSegments<S>,
        cached: &mut Option<(usize, &'a Rows<S>)>,
        index: usize,
    ) -> Cow<'a, Record<S>> {
        if cached.is_none_or(|(base, rows)| index < base || index >= base + rows.len()) {
            *cached = Some(
                records
                    .root
                    .as_ref()
                    .expect("nonempty prefix")
                    .leaf_at(index, 0),
            );
        }
        let (base, rows) = cached.as_ref().unwrap();
        rows.at(index - base)
    }
}
impl<'a, S: StoreSchema> Iterator for RecordIter<'a, S> {
    type Item = Cow<'a, Record<S>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.start == self.end {
            return None;
        }
        let i = self.start;
        self.start += 1;
        Some(Self::at(self.records, &mut self.front, i))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.end - self.start;
        (n, Some(n))
    }
}
impl<S: StoreSchema> DoubleEndedIterator for RecordIter<'_, S> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.start == self.end {
            return None;
        }
        self.end -= 1;
        Some(Self::at(self.records, &mut self.back, self.end))
    }
}
impl<S: StoreSchema> ExactSizeIterator for RecordIter<'_, S> {}
impl<'a, S: StoreSchema> IntoIterator for &'a RecordSegments<S> {
    type Item = Cow<'a, Record<S>>;
    type IntoIter = RecordIter<'a, S>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<'a, S: StoreSchema> IntoIterator for RecordSlice<'a, S> {
    type Item = Cow<'a, Record<S>>;
    type IntoIter = RecordIter<'a, S>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<S: StoreSchema> PartialEq<Vec<Record<S>>> for &RecordSegments<S> {
    fn eq(&self, rhs: &Vec<Record<S>>) -> bool {
        self.len() == rhs.len()
            && self
                .iter()
                .zip(rhs.iter())
                .all(|(own, other)| &*own == other)
    }
}

#[derive(Debug)]
pub enum StoredRow<'a, S: StoreSchema> {
    Encoded(&'a EncodedPayload),
    Decoded(&'a Record<S>),
}
impl<S: StoreSchema> Copy for StoredRow<'_, S> {}
impl<S: StoreSchema> Clone for StoredRow<'_, S> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Debug)]
pub struct RecordRow<'a, S: StoreSchema> {
    rows: &'a Rows<S>,
    offset: usize,
    position: usize,
    commit: u64,
}
impl<S: StoreSchema> Copy for RecordRow<'_, S> {}
impl<S: StoreSchema> Clone for RecordRow<'_, S> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<'a, S: StoreSchema> RecordRow<'a, S> {
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }
    #[must_use]
    pub const fn commit(&self) -> u64 {
        self.commit
    }
    #[must_use]
    pub fn stored(&self) -> StoredRow<'a, S> {
        self.rows.stored(self.offset)
    }
    #[must_use]
    pub fn record(&self) -> Cow<'a, Record<S>> {
        self.rows.at(self.offset)
    }
}

pub struct RowIter<'a, S: StoreSchema> {
    records: &'a RecordSegments<S>,
    start: usize,
    end: usize,
    leaf: Option<(usize, &'a Rows<S>, &'a [u64])>,
}
impl<'a, S: StoreSchema> Iterator for RowIter<'a, S> {
    type Item = RecordRow<'a, S>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.start == self.end {
            return None;
        }
        let index = self.start;
        self.start += 1;
        if self
            .leaf
            .is_none_or(|(base, rows, _)| index < base || index >= base + rows.len())
        {
            self.leaf = Some(
                self.records
                    .root
                    .as_ref()
                    .expect("nonempty prefix")
                    .committed_leaf_at(index, 0),
            );
        }
        let (base, rows, commits) = self.leaf.expect("leaf located");
        Some(RecordRow {
            rows,
            offset: index - base,
            position: index,
            commit: commits.get(index - base).copied().unwrap_or(0),
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.end - self.start;
        (n, Some(n))
    }
}
impl<S: StoreSchema> ExactSizeIterator for RowIter<'_, S> {}
