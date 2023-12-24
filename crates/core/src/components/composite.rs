use crate::{
    cowvec::{
        inflight::{InflightVec, InflightVecWriter},
        CowVec,
    },
    err::Result,
    InflightMatches,
};
use std::sync::Arc;

struct QueueMatch {
    matches: InflightMatches,
    index: usize,
}

struct Queues {
    queues: Vec<QueueMatch>,
}

impl Queues {
    fn new(queues: Vec<InflightMatches>) -> Self {
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

pub struct InflightCompositeRemote(Arc<InflightVecWriter<usize>>);

impl InflightCompositeRemote {
    pub fn compute(self, filters: Vec<InflightMatches>) -> Result<()> {
        let mut queues = Queues::new(filters);

        while let Some(line_number) = queues.take_lowest() {
            if Arc::strong_count(&self.0) < 2 {
                break;
            }
            self.0.write(|inner| {
                if inner.last() == Some(&line_number) {
                    return;
                } else if let Some(&last) = inner.last() {
                    debug_assert!(line_number > last);
                }
                inner.push(line_number)
            });
        }

        self.0.mark_complete();
        Ok(())
    }
}

impl InflightComposite {
    pub fn new() -> (Self, InflightCompositeRemote) {
        let inner = Arc::new(InflightVecWriter::<usize>::new());
        (
            Self(InflightVec::Incomplete(inner.clone())),
            InflightCompositeRemote(inner),
        )
    }

    pub fn empty() -> Self {
        Self(InflightVec::Complete(CowVec::new()))
    }

    pub fn len(&self) -> usize {
        self.0.read(|v| v.len())
    }

    pub fn get(&self, index: usize) -> Option<usize> {
        self.0.read(|v| v.get(index))
    }

    pub fn try_finalize(&mut self) -> bool {
        self.0.try_finalize()
    }
}

pub struct InflightComposite(InflightVec<usize>);
