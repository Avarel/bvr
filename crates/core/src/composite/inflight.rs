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

struct QueueMatch<B> {
    matches: B,
    index: usize,
}

struct Queues<B> {
    queues: Vec<QueueMatch<B>>,
}

impl<B> Queues<B>
where
    B: BufferMatches,
{
    fn new(queues: Vec<B>) -> Self {
        Self {
            queues: queues
                .into_iter()
                .map(|queue| QueueMatch {
                    matches: queue,
                    index: 0,
                })
                .collect(),
        }
    }

    fn take_lowest(&mut self) -> Option<usize> {
        // Take the lowest line number from all the queues
        // and progress the queue that yielded the lowest line number
        'outer: loop {
            let mut min = None;
            for queue in &mut self.queues {
                // We reached the end of this queue but its not complete
                // It could yield a lower line number than all of the other queues
                if queue.index >= queue.matches.len() && !queue.matches.is_complete() {
                    continue 'outer;
                }
                if let Some(ln) = queue.matches.get(queue.index) {
                    if let Some((_, min_ln)) = min {
                        if ln < min_ln {
                            min = Some((&mut queue.index, ln));
                        }
                    } else {
                        min = Some((&mut queue.index, ln));
                    }
                }
            }

            return if let Some((offset, line_number)) = min {
                *offset += 1;
                Some(line_number)
            } else {
                assert!(self.queues.iter().all(|queue| queue.matches.is_complete()));
                None
            };
        }
    }
}

impl InflightImpl<Composite> {
    fn compute<B>(self: Arc<Self>, filters: Vec<B>) -> Result<()>
    where
        B: BufferMatches,
    {
        let mut queues = Queues::new(filters);

        while let Some(line_number) = queues.take_lowest() {
            if Arc::strong_count(&self) < 2 {
                break;
            }
            self.write(|inner| inner.add_line(line_number));
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
