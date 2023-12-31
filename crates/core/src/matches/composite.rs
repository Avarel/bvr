use crate::{cowvec::CowVecWriter, LineSet, Result};
use std::sync::{atomic::AtomicBool, Arc};

struct QueueMatch {
    matches: LineSet,
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
    fn new(queues: Vec<LineSet>, strategy: CompositeStrategy) -> Self {
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
                let Some((offset, line_number)) = self
                    .queues
                    .iter_mut()
                    .filter_map(|queue| queue.peek().map(|ln| (&mut queue.index, ln)))
                    .min_by_key(|(_, ln)| *ln)
                else {
                    assert!(self.queues.iter().all(|queue| queue.matches.is_complete()));
                    return None;
                };
                *offset += 1;
                Some(line_number)
            }
            CompositeStrategy::Intersection => {
                loop {
                    // Take the highest line number from all the queues
                    let Some((queue_index, line_number)) = self
                        .queues
                        .iter_mut()
                        .enumerate()
                        .filter_map(|(i, queue)| queue.peek().map(|ln| (i, ln)))
                        .max_by_key(|&(_, ln)| ln)
                    else {
                        return None;
                    };

                    // Progress all queues that have a line number less than the max
                    for queue in self.queues.iter_mut() {
                        while let Some(ln) = queue.peek() {
                            if ln < line_number {
                                queue.index += 1;
                            } else {
                                break;
                            }
                        }
                    }

                    if self
                        .queues
                        .iter()
                        .all(|queue| queue.peek() == Some(line_number))
                    {
                        self.queues[queue_index].index += 1;
                        return Some(line_number);
                    } else {
                        self.queues[queue_index].index += 1;
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub enum CompositeStrategy {
    Intersection,
    Union,
}

pub(super) struct LineCompositeRemote {
    pub(super) buf: CowVecWriter<usize>,
    pub(super) completed: Arc<AtomicBool>,
    pub(super) strategy: CompositeStrategy,
}

impl LineCompositeRemote {
    pub fn compose(mut self, filters: Vec<LineSet>) -> Result<()> {
        let len = filters.iter().map(|filter| filter.len()).sum::<usize>();
        // Divide by 2 because there may be a lot of overlap, but hopefully this
        // nonscientific guess is good enough
        // In the common case, we only have 1 reallocation so its not too bad
        self.buf.reserve(len / 2);

        let mut queues = Queues::new(filters, self.strategy);

        while let Some(line_number) = queues.take_lowest() {
            if !self.has_readers() {
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

    pub fn has_readers(&self) -> bool {
        Arc::strong_count(&self.completed) > 1
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
    use super::CompositeStrategy;
    use crate::LineSet;

    #[test]
    fn test_composite_union_basic() {
        let matches1 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineSet::from(vec![1, 3, 5, 7, 9]);

        let composite =
            LineSet::compose(vec![matches1, matches2], true, CompositeStrategy::Union).unwrap();

        let result = vec![1, 2, 3, 4, 5, 7, 9];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }

    #[test]
    fn test_composite_union_disjoint() {
        let matches1 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineSet::from(vec![6, 7, 8, 9, 10]);

        let composite =
            LineSet::compose(vec![matches1, matches2], true, CompositeStrategy::Union).unwrap();

        let result = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }

    #[test]
    fn test_composite_union_same() {
        let matches1 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineSet::from(vec![1, 2, 3, 4, 5]);

        let composite =
            LineSet::compose(vec![matches1, matches2], true, CompositeStrategy::Union).unwrap();

        let result = vec![1, 2, 3, 4, 5];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }

    #[test]
    fn test_composite_union_empty() {
        let matches1 = LineSet::from(vec![]);
        let matches2 = LineSet::from(vec![1, 2, 3, 4, 5]);

        let composite =
            LineSet::compose(vec![matches1, matches2], true, CompositeStrategy::Union).unwrap();

        let result = vec![1, 2, 3, 4, 5];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }

    #[test]
    fn test_composite_union_empty_all() {
        let matches1 = LineSet::from(vec![]);
        let matches2 = LineSet::from(vec![]);

        let composite =
            LineSet::compose(vec![matches1, matches2], true, CompositeStrategy::Union).unwrap();

        let inner = composite.into_inner();
        assert_eq!(0, inner.len());
    }

    #[test]
    fn test_composite_union_empty_all_but_one() {
        let matches1 = LineSet::from(vec![]);
        let matches2 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches3 = LineSet::from(vec![]);

        let composite = LineSet::compose(
            vec![matches1, matches2, matches3],
            true,
            CompositeStrategy::Union,
        )
        .unwrap();

        let result = vec![1, 2, 3, 4, 5];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }

    #[test]
    fn test_composite_union_many() {
        let matches1 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineSet::from(vec![1, 3, 5, 7, 9]);
        let matches3 = LineSet::from(vec![2, 4, 6, 8, 10]);
        let matches4 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches5 = LineSet::from(vec![1, 3, 5, 7, 9]);
        let matches6 = LineSet::from(vec![2, 4, 6, 8, 10]);
        let matches7 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches8 = LineSet::from(vec![1, 3, 5, 7, 9]);
        let matches9 = LineSet::from(vec![2, 4, 6, 8, 10]);
        let matches10 = LineSet::from(vec![1, 2, 3, 4, 5]);

        let composite = LineSet::compose(
            vec![
                matches1, matches2, matches3, matches4, matches5, matches6, matches7, matches8,
                matches9, matches10,
            ],
            true,
            CompositeStrategy::Union,
        )
        .unwrap();

        let result = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }

    #[test]
    fn test_composite_union_many_disjoint() {
        let matches1 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineSet::from(vec![6, 7, 8, 9, 10]);
        let matches3 = LineSet::from(vec![11, 12, 13, 14, 15]);
        let matches4 = LineSet::from(vec![16, 17, 18, 19, 20]);
        let matches5 = LineSet::from(vec![21, 22, 23, 24, 25]);
        let matches6 = LineSet::from(vec![26, 27, 28, 29, 30]);
        let matches7 = LineSet::from(vec![31, 32, 33, 34, 35]);
        let matches8 = LineSet::from(vec![36, 37, 38, 39, 40]);
        let matches9 = LineSet::from(vec![41, 42, 43, 44, 45]);
        let matches10 = LineSet::from(vec![46, 47, 48, 49, 50]);

        let composite = LineSet::compose(
            vec![
                matches1, matches2, matches3, matches4, matches5, matches6, matches7, matches8,
                matches9, matches10,
            ],
            true,
            CompositeStrategy::Union,
        )
        .unwrap();

        let result = vec![
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, //
            11, 12, 13, 14, 15, 16, 17, 18, 19, 20, //
            21, 22, 23, 24, 25, 26, 27, 28, 29, 30, //
            31, 32, 33, 34, 35, 36, 37, 38, 39, 40, //
            41, 42, 43, 44, 45, 46, 47, 48, 49, 50, //
        ];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }

    #[test]
    fn test_composite_intersection_basic() {
        let matches1 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineSet::from(vec![1, 3, 5, 7, 9]);

        let composite = LineSet::compose(
            vec![matches1, matches2],
            true,
            CompositeStrategy::Intersection,
        )
        .unwrap();

        let result = vec![1, 3, 5];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }

    #[test]
    fn test_composite_intersection_disjoint() {
        let matches1 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineSet::from(vec![6, 7, 8, 9, 10]);

        let composite = LineSet::compose(
            vec![matches1, matches2],
            true,
            CompositeStrategy::Intersection,
        )
        .unwrap();

        let inner = composite.into_inner();
        assert_eq!(0, inner.len());
    }

    #[test]
    fn test_composite_intersection_same() {
        let matches1 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineSet::from(vec![1, 2, 3, 4, 5]);

        let composite = LineSet::compose(
            vec![matches1, matches2],
            true,
            CompositeStrategy::Intersection,
        )
        .unwrap();

        let result = vec![1, 2, 3, 4, 5];

        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }

    #[test]
    fn test_composite_intersection_empty() {
        let matches1 = LineSet::from(vec![]);
        let matches2 = LineSet::from(vec![1, 2, 3, 4, 5]);

        let composite = LineSet::compose(
            vec![matches1, matches2],
            true,
            CompositeStrategy::Intersection,
        )
        .unwrap();

        let inner = composite.into_inner();
        assert_eq!(0, inner.len());
    }

    #[test]
    fn test_composite_intersection_empty_all() {
        let matches1 = LineSet::from(vec![]);
        let matches2 = LineSet::from(vec![]);

        let composite = LineSet::compose(
            vec![matches1, matches2],
            true,
            CompositeStrategy::Intersection,
        )
        .unwrap();

        let inner = composite.into_inner();
        assert_eq!(0, inner.len());
    }

    #[test]
    fn test_composite_intersection_empty_all_but_one() {
        let matches1 = LineSet::from(vec![]);
        let matches2 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches3 = LineSet::from(vec![]);

        let composite = LineSet::compose(
            vec![matches1, matches2, matches3],
            true,
            CompositeStrategy::Intersection,
        )
        .unwrap();

        let inner = composite.into_inner();
        assert_eq!(0, inner.len());
    }

    #[test]
    fn test_composite_intersection_many() {
        let matches1 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineSet::from(vec![1, 3, 5, 7, 9]);
        let matches3 = LineSet::from(vec![2, 4, 6, 8, 10]);
        let matches4 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches5 = LineSet::from(vec![1, 3, 5, 7, 9]);
        let matches6 = LineSet::from(vec![2, 4, 6, 8, 10]);
        let matches7 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches8 = LineSet::from(vec![1, 3, 5, 7, 9]);
        let matches9 = LineSet::from(vec![2, 4, 6, 8, 10]);
        let matches10 = LineSet::from(vec![1, 2, 3, 4, 5]);

        let composite = LineSet::compose(
            vec![
                matches1, matches2, matches3, matches4, matches5, matches6, matches7, matches8,
                matches9, matches10,
            ],
            true,
            CompositeStrategy::Intersection,
        )
        .unwrap();

        let inner = composite.into_inner();
        assert_eq!(0, inner.len());
    }

    #[test]
    fn test_composite_intersection_many_disjoint() {
        let matches1 = LineSet::from(vec![1, 2, 3, 4, 5]);
        let matches2 = LineSet::from(vec![6, 7, 8, 9, 10]);
        let matches3 = LineSet::from(vec![11, 12, 13, 14, 15]);
        let matches4 = LineSet::from(vec![16, 17, 18, 19, 20]);
        let matches5 = LineSet::from(vec![21, 22, 23, 24, 25]);
        let matches6 = LineSet::from(vec![26, 27, 28, 29, 30]);
        let matches7 = LineSet::from(vec![31, 32, 33, 34, 35]);
        let matches8 = LineSet::from(vec![36, 37, 38, 39, 40]);
        let matches9 = LineSet::from(vec![41, 42, 43, 44, 45]);
        let matches10 = LineSet::from(vec![46, 47, 48, 49, 50]);

        let composite = LineSet::compose(
            vec![
                matches1, matches2, matches3, matches4, matches5, matches6, matches7, matches8,
                matches9, matches10,
            ],
            true,
            CompositeStrategy::Intersection,
        )
        .unwrap();

        let inner = composite.into_inner();
        assert_eq!(0, inner.len());
    }

    #[test]
    fn test_composite_intersection_many_some() {
        let matches1 = LineSet::from(vec![1, 2, 3, 4, 5, 10, 20, 30, 40, 50, 51, 213]);
        let matches2 = LineSet::from(vec![6, 7, 8, 9, 10, 15, 20, 30, 40, 45, 50, 51]);
        let matches3 = LineSet::from(vec![10, 11, 12, 13, 14, 15, 20, 30, 35, 40, 50, 60]);
        let matches4 = LineSet::from(vec![10, 16, 17, 18, 19, 20, 30, 40, 50, 51, 52, 53]);
        let matches5 = LineSet::from(vec![10, 20, 21, 22, 23, 24, 25, 30, 32, 33, 35, 40, 50]);
        let matches6 = LineSet::from(vec![2, 10, 20, 26, 27, 28, 29, 30, 40, 50, 51, 52, 53]);
        let matches7 = LineSet::from(vec![10, 20, 30, 31, 32, 33, 34, 35, 40, 50, 51, 52, 53]);
        let matches8 = LineSet::from(vec![10, 20, 25, 30, 36, 37, 38, 39, 40, 45, 50]);
        let matches9 = LineSet::from(vec![4, 10, 20, 30, 40, 41, 42, 43, 44, 45, 50, 51, 52, 53]);
        let matches10 = LineSet::from(vec![1, 10, 20, 30, 40, 46, 47, 48, 49, 50]);

        let composite = LineSet::compose(
            vec![
                matches1, matches2, matches3, matches4, matches5, matches6, matches7, matches8,
                matches9, matches10,
            ],
            true,
            CompositeStrategy::Intersection,
        )
        .unwrap();

        let result = vec![10, 20, 30, 40, 50];
        let inner = composite.into_inner();
        for i in 0..result.len() {
            assert_eq!(result.get(i).copied(), inner.get(i));
        }
    }
}
