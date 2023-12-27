use crate::{cowvec::CowVecWriter, LineMatches, Result};
use std::sync::{atomic::AtomicBool, Arc};

struct QueueMatch {
    matches: LineMatches,
    index: usize,
}

impl QueueMatch {
    fn is_ready(&self) -> bool {
        // We reached the end of this queue but its not complete
        self.matches.is_complete() || self.index < self.matches.len()
    }

    fn peek(&self) -> Option<usize> {
        while !self.is_ready() {
            // Opportunistically spin while we wait for the queue to be ready
            std::hint::spin_loop();
        }
        self.matches.get(self.index)
    }
}

struct Queues {
    queues: Vec<QueueMatch>,
    strategy: CompositeStrategy,
}

impl Queues {
    fn new(queues: Vec<LineMatches>, strategy: CompositeStrategy) -> Self {
        Self {
            queues: queues
                .into_iter()
                .map(|queue| QueueMatch {
                    matches: queue,
                    index: 0,
                })
                .collect(),
            strategy,
        }
    }

    fn take_lowest(&mut self) -> Option<usize> {
        match self.strategy {
            CompositeStrategy::Union => {
                // Take the lowest line number from all the queues
                // and progress the queue that yielded the lowest line number
                let mut min = None;
                for queue in self.queues.iter_mut() {
                    if let Some(ln) = queue.peek() {
                        if let Some((_, min_ln)) = min {
                            if ln < min_ln {
                                min = Some((&mut queue.index, ln));
                            }
                        } else {
                            min = Some((&mut queue.index, ln));
                        }
                    }
                }

                if let Some((offset, line_number)) = min {
                    *offset += 1;
                    Some(line_number)
                } else {
                    assert!(self.queues.iter().all(|queue| queue.matches.is_complete()));
                    None
                }
            }
            CompositeStrategy::Intersection => {
                unimplemented!("Intersection strategy is not implemented")
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum CompositeStrategy {
    #[allow(dead_code)]
    Intersection,
    Union,
}

pub(super) struct LineCompositeRemote {
    pub(super) buf: CowVecWriter<usize>,
    pub(super) completed: Arc<AtomicBool>,
    pub(super) strategy: CompositeStrategy,
}

impl LineCompositeRemote {
    pub fn compute(mut self, filters: Vec<LineMatches>) -> Result<()> {
        let len = filters.iter().map(|filter| filter.len()).sum::<usize>();
        // Divide by 2 because there may be a lot of overlap, but hopefully this
        // nonscientific guess is good enough
        // In the common case, we only have 1 reallocation so its not too bad
        self.buf.reserve(len / 2);

        let mut queues = Queues::new(filters, self.strategy);

        while let Some(line_number) = queues.take_lowest() {
            if !self.buf.has_readers() {
                break;
            }

            if let Some(&last) = self.buf.last() {
                if last == line_number {
                    continue;
                }
                debug_assert!(line_number > last);
            }
            self.buf.push(line_number);
        }
        Ok(())
    }
}

impl Drop for LineCompositeRemote {
    fn drop(&mut self) {
        self.completed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use crate::LineMatches;

    #[test]
    fn test_composite_basic() {
        let matches1 = LineMatches::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineMatches::from(vec![1, 3, 5, 7, 9]);

        let composite = LineMatches::compose(vec![matches1, matches2], true).unwrap();

        let result = vec![1, 2, 3, 4, 5, 7, 9];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }

    #[test]
    fn test_composite_disjoint() {
        let matches1 = LineMatches::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineMatches::from(vec![6, 7, 8, 9, 10]);

        let composite = LineMatches::compose(vec![matches1, matches2], true).unwrap();

        let result = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }

    #[test]
    fn test_composite_same() {
        let matches1 = LineMatches::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineMatches::from(vec![1, 2, 3, 4, 5]);

        let composite = LineMatches::compose(vec![matches1, matches2], true).unwrap();

        let result = vec![1, 2, 3, 4, 5];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }
}
