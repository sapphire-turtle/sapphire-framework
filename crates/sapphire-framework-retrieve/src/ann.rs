//! An in-memory HNSW index over the store's vectors (`instant-distance`), for approximate
//! nearest-neighbour search once the store holds many of them.
//!
//! The index is derived from the redb `vectors` table, like the tantivy index, and never
//! saved. It is built on a background thread from a snapshot of the table; vectors that
//! change afterwards are tracked by id, searched exactly next to the index, and filtered
//! out of its results. When they reach a tenth of the index, it is built again.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use instant_distance::{Builder, HnswMap, Search};

/// Below this many vectors, search stays brute force.
pub const ANN_MIN: usize = 10_000;
/// Candidates the index returns per search; a search that wants more is brute force.
pub const EF_SEARCH: usize = 200;

/// A vector as the index holds it.
#[derive(Clone)]
pub struct Point(pub Vec<f32>);

impl instant_distance::Point for Point {
    fn distance(&self, other: &Self) -> f32 {
        self.0
            .iter()
            .zip(&other.0)
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f32>()
            .sqrt()
    }
}

/// A finished index, as of the snapshot it was built from.
struct Built {
    map: HnswMap<Point, i64>,
    len: usize,
}

#[derive(Default)]
struct State {
    built: Option<Arc<Built>>,
    building: bool,
    /// Bumped by [`Ann::clear`]: a build started before it is thrown away.
    generation: u64,
    /// Bumped by every change.
    seq: u64,
    /// The ids changed since the snapshot of the current index, with the `seq` of their
    /// last change.
    changed: HashMap<i64, u64>,
}

/// What a build started from, handed back to [`Ann::finish`].
#[derive(Clone, Copy, Debug)]
pub struct Ticket {
    generation: u64,
    seq: u64,
}

/// The index, if any, and what has changed since it was built.
pub struct Ann {
    state: Mutex<State>,
    min: usize,
}

/// What [`Ann::search`] found: index candidates, and the ids to search exactly.
pub struct Candidates {
    /// `(distance, id)`, nearest first, without the changed ids.
    pub nearest: Vec<(f64, i64)>,
    /// Changed since the build: search these exactly (their vectors may be gone).
    pub changed: Vec<i64>,
}

impl Ann {
    /// An empty index that starts building at `min` vectors.
    pub fn new(min: usize) -> Ann {
        Ann {
            state: Mutex::new(State::default()),
            min,
        }
    }

    /// The number of vectors at which a build starts.
    pub fn min(&self) -> usize {
        self.min
    }

    /// The vector of `id` was added, replaced or removed.
    pub fn changed(&self, id: i64) {
        let mut s = self.state.lock().unwrap();
        if s.built.is_some() || s.building {
            s.seq += 1;
            let seq = s.seq;
            s.changed.insert(id, seq);
        }
    }

    /// Every vector was dropped: forget the index and any build in flight.
    pub fn clear(&self) {
        let mut s = self.state.lock().unwrap();
        let generation = s.generation + 1;
        *s = State {
            generation,
            ..State::default()
        };
    }

    /// Whether a build should start for a store of `count` vectors: none is running, and
    /// there is no index yet or a tenth of it has changed. The caller must then build and
    /// call [`finish`](Self::finish), whatever the outcome.
    pub fn start(&self, count: usize) -> Option<Ticket> {
        let mut s = self.state.lock().unwrap();
        if count < self.min || s.building {
            return None;
        }
        let stale = match &s.built {
            None => true,
            Some(b) => s.changed.len() * 10 > b.len.max(1),
        };
        if !stale {
            return None;
        }
        s.building = true;
        Some(Ticket {
            generation: s.generation,
            seq: s.seq,
        })
    }

    /// Install what a build made from a snapshot taken after [`start`](Self::start); `None`
    /// when it failed.
    pub fn finish(&self, ticket: Ticket, index: Option<(Vec<Point>, Vec<i64>)>) {
        let built = index.map(|(points, ids)| {
            let len = ids.len();
            let map = Builder::default().ef_search(EF_SEARCH).build(points, ids);
            Built { map, len }
        });
        let mut s = self.state.lock().unwrap();
        if s.generation != ticket.generation {
            return;
        }
        s.building = false;
        if let Some(built) = built {
            s.built = Some(Arc::new(built));
            // A change made after the snapshot may or may not be in it: keep searching it
            // exactly.
            s.changed.retain(|_, seq| *seq > ticket.seq);
        }
    }

    /// The nearest `want` candidates for `query` from the index, or `None` without one.
    pub fn search(&self, query: &[f32], want: usize) -> Option<Candidates> {
        let (built, changed) = {
            let s = self.state.lock().unwrap();
            let built = s.built.clone()?;
            let changed: HashSet<i64> = s.changed.keys().copied().collect();
            (built, changed)
        };
        let mut search = Search::default();
        let nearest = built
            .map
            .search(&Point(query.to_vec()), &mut search)
            .filter(|item| !changed.contains(item.value))
            .take(want)
            .map(|item| (item.distance as f64, *item.value))
            .collect();
        Some(Candidates {
            nearest,
            changed: changed.into_iter().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random unit vectors.
    fn vectors(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
        let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (0..n)
            .map(|_| {
                let v: Vec<f32> = (0..dim)
                    .map(|_| {
                        x = x
                            .wrapping_mul(6364136223846793005)
                            .wrapping_add(1442695040888963407);
                        ((x >> 33) as f32 / (1u64 << 31) as f32) - 0.5
                    })
                    .collect();
                let norm = v.iter().map(|a| a * a).sum::<f32>().sqrt();
                v.into_iter().map(|a| a / norm).collect()
            })
            .collect()
    }

    fn exact(all: &[Vec<f32>], q: &[f32], k: usize) -> Vec<i64> {
        let mut d: Vec<(f32, i64)> = all
            .iter()
            .enumerate()
            .map(|(i, v)| {
                (
                    instant_distance::Point::distance(&Point(v.clone()), &Point(q.to_vec())),
                    i as i64,
                )
            })
            .collect();
        d.sort_by(|a, b| a.0.total_cmp(&b.0));
        d.into_iter().take(k).map(|(_, i)| i).collect()
    }

    fn build(ann: &Ann, all: &[Vec<f32>]) {
        let ticket = ann.start(all.len()).expect("a build starts");
        let points = all.iter().cloned().map(Point).collect();
        let ids = (0..all.len() as i64).collect();
        ann.finish(ticket, Some((points, ids)));
    }

    #[test]
    fn small_stores_never_build() {
        let ann = Ann::new(100);
        assert!(ann.start(99).is_none());
        assert!(ann.search(&[0.0; 4], 10).is_none());
    }

    #[test]
    fn recall_against_brute_force() {
        let all = vectors(3000, 32, 1);
        let ann = Ann::new(1000);
        build(&ann, &all);
        let queries = vectors(20, 32, 2);
        let mut hits = 0;
        for q in &queries {
            let want = exact(&all, q, 10);
            let got: Vec<i64> = ann
                .search(q, 10)
                .unwrap()
                .nearest
                .iter()
                .map(|c| c.1)
                .collect();
            hits += want.iter().filter(|id| got.contains(id)).count();
        }
        let recall = hits as f64 / (queries.len() * 10) as f64;
        assert!(recall > 0.9, "recall {recall}");
    }

    #[test]
    fn changed_ids_leave_the_index_until_the_next_build() {
        let all = vectors(200, 8, 3);
        let ann = Ann::new(10);
        build(&ann, &all);
        // Vector 5 is its own nearest neighbour; once changed, the index no longer offers it.
        ann.changed(5);
        let c = ann.search(&all[5], 5).unwrap();
        assert!(!c.nearest.iter().any(|n| n.1 == 5));
        assert_eq!(c.changed, vec![5]);
        // One change in 200 is not stale; 21 are.
        assert!(ann.start(200).is_none());
        for id in 100..120 {
            ann.changed(id);
        }
        let ticket = ann.start(200).expect("a tenth changed");
        // While that build runs, another change comes in after its snapshot.
        ann.changed(7);
        let points = all.iter().cloned().map(Point).collect();
        ann.finish(ticket, Some((points, (0..200).collect())));
        assert_eq!(ann.search(&all[0], 5).unwrap().changed, vec![7]);
    }

    #[test]
    fn a_clear_discards_the_index_and_a_build_in_flight() {
        let all = vectors(50, 8, 4);
        let ann = Ann::new(10);
        build(&ann, &all);
        let ticket = {
            for id in 0..10 {
                ann.changed(id);
            }
            ann.start(50).unwrap()
        };
        ann.clear();
        let points = all.iter().cloned().map(Point).collect();
        ann.finish(ticket, Some((points, (0..50).collect())));
        assert!(ann.search(&all[0], 5).is_none());
        assert!(
            ann.start(50).is_some(),
            "a new build may start after the clear"
        );
    }
}
