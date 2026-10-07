use std::sync::Arc;

const LEAF: usize = 128;

#[derive(Clone, Debug, Default)]
pub(crate) struct QueryPositions {
    full: Option<Arc<Segment>>,
    tail: Option<Arc<[(u64, usize)]>>,
}

#[derive(Debug)]
enum Segment {
    Leaf(Box<[(u64, usize)]>),
    Branch {
        left: Arc<Self>,
        right: Arc<Self>,
        height: usize,
        last: u64,
    },
}

impl Segment {
    fn height(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Branch { height, .. } => *height,
        }
    }
    fn last(&self) -> u64 {
        match self {
            Self::Leaf(coordinates) => {
                coordinates
                    .last()
                    .expect("a leaf holds at least one coordinate")
                    .0
            }
            Self::Branch { last, .. } => *last,
        }
    }
    fn branch(left: Arc<Self>, right: Arc<Self>) -> Arc<Self> {
        Arc::new(Self::Branch {
            height: 1 + left.height().max(right.height()),
            last: right.last(),
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

impl QueryPositions {
    pub(crate) fn append(&mut self, batch: Vec<(u64, usize)>) {
        if batch.is_empty() {
            return;
        }
        let coordinates = match self.tail.take() {
            Some(tail) => {
                let mut joined = Vec::with_capacity(tail.len() + batch.len());
                joined.extend_from_slice(&tail);
                joined.extend(batch);
                joined
            }
            None => batch,
        };
        let mut leaves = coordinates.chunks_exact(LEAF);
        for leaf in &mut leaves {
            let leaf = Arc::new(Segment::Leaf(leaf.into()));
            self.full = Some(match self.full.take() {
                Some(root) => Segment::append(root, leaf),
                None => leaf,
            });
        }
        let rest = leaves.remainder();
        self.tail = (!rest.is_empty()).then(|| Arc::from(rest));
    }
    pub(crate) fn end(&self) -> u64 {
        let last = match &self.tail {
            Some(tail) => tail.last().map(|(ordinal, _)| *ordinal),
            None => self.full.as_ref().map(|root| root.last()),
        };
        last.map_or(0, |ordinal| {
            ordinal
                .checked_add(1)
                .expect("record ordinal has a successor")
        })
    }
    pub(crate) fn first(&self) -> Option<u64> {
        let mut node = self.full.as_deref();
        while let Some(current) = node {
            match current {
                Segment::Branch { left, .. } => node = Some(left),
                Segment::Leaf(coordinates) => return coordinates.first().map(|(o, _)| *o),
            }
        }
        self.tail
            .as_deref()
            .and_then(|tail| tail.first())
            .map(|(o, _)| *o)
    }
    pub(crate) fn since(&self, from: u64) -> impl Iterator<Item = usize> + '_ {
        self.coordinates_since(from).map(|(_, at)| at)
    }
    pub(crate) fn coordinates_since(&self, from: u64) -> impl Iterator<Item = (u64, usize)> + '_ {
        let tail = self.tail.as_deref().unwrap_or(&[]);
        let start = tail.partition_point(|(ordinal, _)| *ordinal < from);
        Since::new(self.full.as_deref(), from).chain(tail[start..].iter().copied())
    }
}

struct Since<'a> {
    pending: Vec<&'a Segment>,
    leaf: std::slice::Iter<'a, (u64, usize)>,
}

impl<'a> Since<'a> {
    fn new(root: Option<&'a Segment>, from: u64) -> Self {
        let mut pending = Vec::new();
        let mut leaf: &'a [(u64, usize)] = &[];
        let mut node = root;
        while let Some(current) = node {
            match current {
                Segment::Branch { left, right, .. } => {
                    if left.last() < from {
                        node = Some(right);
                    } else {
                        pending.push(&**right);
                        node = Some(left);
                    }
                }
                Segment::Leaf(coordinates) => {
                    let start = coordinates.partition_point(|(ordinal, _)| *ordinal < from);
                    leaf = &coordinates[start..];
                    node = None;
                }
            }
        }
        Self {
            pending,
            leaf: leaf.iter(),
        }
    }
}

impl Iterator for Since<'_> {
    type Item = (u64, usize);
    fn next(&mut self) -> Option<(u64, usize)> {
        loop {
            if let Some(coordinate) = self.leaf.next() {
                return Some(*coordinate);
            }
            let mut node = self.pending.pop()?;
            loop {
                match node {
                    Segment::Branch { left, right, .. } => {
                        self.pending.push(right);
                        node = left;
                    }
                    Segment::Leaf(coordinates) => {
                        self.leaf = coordinates.iter();
                        break;
                    }
                }
            }
        }
    }
}

