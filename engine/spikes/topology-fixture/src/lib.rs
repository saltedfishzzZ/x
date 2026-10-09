//! TS03：平台感知的离散拓扑实验。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct HeightGraph {
    pub heights: Vec<f64>,
    pub edges: Vec<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CriticalKind {
    Minimum,
    Maximum,
    Saddle,
    Regular,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlateauComponent {
    pub id: usize,
    pub vertices: Vec<usize>,
    pub minimum_height: f64,
    pub maximum_height: f64,
    pub lower_neighbours: Vec<usize>,
    pub upper_neighbours: Vec<usize>,
    pub kind: CriticalKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MergeEvent {
    pub plateau_id: usize,
    pub merged_roots: Vec<usize>,
}

#[derive(Clone, Debug)]
struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
        }
    }

    fn find(&mut self, value: usize) -> usize {
        if self.parent[value] != value {
            self.parent[value] = self.find(self.parent[value]);
        }
        self.parent[value]
    }

    fn union(&mut self, a: usize, b: usize) {
        let root_a = self.find(a);
        let root_b = self.find(b);
        if root_a != root_b {
            let (keep, merge) = if root_a < root_b {
                (root_a, root_b)
            } else {
                (root_b, root_a)
            };
            self.parent[merge] = keep;
        }
    }
}

#[must_use]
pub fn plateau_components(graph: &HeightGraph, tolerance: f64) -> Vec<PlateauComponent> {
    let mut vertices = DisjointSet::new(graph.heights.len());
    for &(a, b) in &graph.edges {
        if (graph.heights[a] - graph.heights[b]).abs() <= tolerance {
            vertices.union(a, b);
        }
    }

    let mut grouped: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for vertex in 0..graph.heights.len() {
        let root = vertices.find(vertex);
        grouped.entry(root).or_default().push(vertex);
    }
    let root_to_id: BTreeMap<usize, usize> = grouped
        .keys()
        .enumerate()
        .map(|(id, root)| (*root, id))
        .collect();
    let mut vertex_to_component = vec![0; graph.heights.len()];
    for (&root, members) in &grouped {
        for &vertex in members {
            vertex_to_component[vertex] = root_to_id[&root];
        }
    }

    let mut lower = vec![BTreeSet::new(); grouped.len()];
    let mut upper = vec![BTreeSet::new(); grouped.len()];
    for &(a, b) in &graph.edges {
        let ca = vertex_to_component[a];
        let cb = vertex_to_component[b];
        if ca == cb {
            continue;
        }
        if graph.heights[a] < graph.heights[b] - tolerance {
            upper[ca].insert(cb);
            lower[cb].insert(ca);
        } else if graph.heights[b] < graph.heights[a] - tolerance {
            upper[cb].insert(ca);
            lower[ca].insert(cb);
        }
    }

    grouped
        .into_iter()
        .map(|(root, members)| {
            let id = root_to_id[&root];
            let minimum_height = members
                .iter()
                .map(|&v| graph.heights[v])
                .fold(f64::INFINITY, f64::min);
            let maximum_height = members
                .iter()
                .map(|&v| graph.heights[v])
                .fold(f64::NEG_INFINITY, f64::max);
            let kind = if lower[id].is_empty() {
                CriticalKind::Minimum
            } else if upper[id].is_empty() {
                CriticalKind::Maximum
            } else if lower[id].len() >= 2 && upper[id].len() >= 2 {
                CriticalKind::Saddle
            } else {
                CriticalKind::Regular
            };
            PlateauComponent {
                id,
                vertices: members,
                minimum_height,
                maximum_height,
                lower_neighbours: lower[id].iter().copied().collect(),
                upper_neighbours: upper[id].iter().copied().collect(),
                kind,
            }
        })
        .collect()
}

#[must_use]
pub fn sublevel_merge_events(components: &[PlateauComponent]) -> Vec<MergeEvent> {
    let mut order: Vec<_> = (0..components.len()).collect();
    order.sort_by(|&a, &b| {
        components[a]
            .minimum_height
            .total_cmp(&components[b].minimum_height)
            .then_with(|| a.cmp(&b))
    });
    let mut sets = DisjointSet::new(components.len());
    let mut active = vec![false; components.len()];
    let mut events = Vec::new();
    for id in order {
        active[id] = true;
        let mut roots = BTreeSet::new();
        for &lower in &components[id].lower_neighbours {
            if active[lower] {
                roots.insert(sets.find(lower));
            }
        }
        if roots.len() > 1 {
            events.push(MergeEvent {
                plateau_id: id,
                merged_roots: roots.iter().copied().collect(),
            });
        }
        for root in roots {
            sets.union(id, root);
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_plateau_remains_one_region() {
        let graph = HeightGraph {
            heights: vec![0.0, 1.0, 1.0, 0.0],
            edges: vec![(0, 1), (1, 2), (2, 3)],
        };
        let components = plateau_components(&graph, 1.0e-12);
        assert_eq!(components.len(), 3);
        assert!(components.iter().any(|c| c.vertices == vec![1, 2]));
    }

    #[test]
    fn saddle_merges_two_lower_components() {
        // Two minima meet at vertex 2; two higher branches make it a saddle.
        let graph = HeightGraph {
            heights: vec![0.0, 0.0, 1.0, 2.0, 2.0],
            edges: vec![(0, 2), (1, 2), (2, 3), (2, 4)],
        };
        let components = plateau_components(&graph, 1.0e-12);
        let saddle = components
            .iter()
            .find(|c| c.kind == CriticalKind::Saddle)
            .unwrap();
        assert_eq!(saddle.lower_neighbours.len(), 2);
        assert_eq!(saddle.upper_neighbours.len(), 2);
        let events = sublevel_merge_events(&components);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].merged_roots.len(), 2);
    }

    #[test]
    fn equal_minima_stay_distinct_without_an_edge() {
        let graph = HeightGraph {
            heights: vec![0.0, 0.0, 1.0],
            edges: vec![(0, 2), (1, 2)],
        };
        let components = plateau_components(&graph, 0.0);
        assert_eq!(
            components
                .iter()
                .filter(|c| c.kind == CriticalKind::Minimum)
                .count(),
            2
        );
    }
}
