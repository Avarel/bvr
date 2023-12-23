use std::sync::Arc;

use crate::{
    err::Result,
    inflight_tool::{Inflight, InflightImpl, Inflightable},
    matches::BufferMatches,
};

use super::{Composite, IncompleteComposite};

impl Inflightable for Composite {
    type Incomplete = IncompleteComposite;

    type Remote = InflightCompositeRemote;

    fn make_remote(inner: Arc<crate::inflight_tool::InflightImpl<Self>>) -> Self::Remote {
        InflightCompositeRemote(inner)
    }

    fn finish(inner: Self::Incomplete) -> Self {
        inner.finish()
    }

    fn snapshot(inner: &Self::Incomplete) -> Self {
        inner.inner.clone()
    }
}

impl InflightImpl<Composite> {
    fn compute<B>(self: Arc<Self>, filters: Vec<B>) -> Result<()>
    where
        B: BufferMatches,
    {
        let mut filters = filters.into_iter().map(|v| (0, v)).collect::<Vec<_>>();

        'outer: while Arc::strong_count(&self) >= 2 {
            let mut min = None;
            for (offset, filter) in &mut filters {
                // We have in progress pending searches, and they could yield something
                // thats lower than every other search thats ready to yield a result
                if *offset >= filter.len() && !filter.is_complete() {
                    continue 'outer;
                }
                if let Some(ln) = filter.get(*offset) {
                    if let Some((_, min_ln)) = min {
                        if ln < min_ln {
                            min = Some((offset, ln));
                        }
                    } else {
                        min = Some((offset, ln));
                    }
                }
            }

            if let Some((offset, line_number)) = min {
                *offset += 1;
                self.write(|inner| inner.add_line(line_number));
            } else if filters.iter().all(|(_, filter)| filter.is_complete()) {
                break;
            }
        }
        self.mark_complete();
        Ok(())
    }
}

pub struct InflightCompositeRemote(Arc<InflightImpl<Composite>>);

impl InflightCompositeRemote {
    pub fn compute<B>(self, filters: Vec<B>) -> Result<()>
    where
        B: BufferMatches,
    {
        self.0.compute(filters)
    }
}

impl Inflight<Composite> {
    pub fn empty() -> Self {
        Self::Complete(Composite::empty())
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Incomplete(inner) => inner.read(|v| v.len()),
            Self::Complete(inner) => inner.len(),
        }
    }

    pub fn get(&self, index: usize) -> Option<usize> {
        match self {
            Self::Incomplete(inner) => inner.read(|v| v.get(index)),
            Self::Complete(inner) => inner.get(index),
        }
    }
}

pub type InflightComposite = Inflight<Composite>;
