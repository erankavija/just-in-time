//! Acyclicity over symbolic keyed graphs.
//!
//! [`DependencyGraph`](super::DependencyGraph) needs nodes that carry a real id.
//! Callers that reason about a graph before it exists — batch creation over
//! symbolic keys, breakdown over child indices, template roles — hold nothing
//! but an adjacency list. [`find_keyed_cycle`] serves those: it is generic over
//! any `(K, Vec<K>)` adjacency and reports the offending path.

use std::collections::HashMap;
use std::hash::Hash;

/// DFS colouring: a key on the active path, or one whose subtree is exhausted.
#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Visiting,
    Done,
}

/// Find one cycle in a keyed adjacency list, or `None` when it is acyclic
/// (`@/inv/dag-acyclic`).
///
/// `adjacency` pairs each key with the keys it points at. Edges whose target is
/// absent from `adjacency` are ignored, so an unresolved reference stays an
/// unresolved reference rather than being reported as a cycle. When a key
/// repeats, its last entry supplies the edges.
///
/// The cycle is returned as a closed path in traversal order: the first key
/// repeats as the last, so `[a, b, a]` reads `a → b → a` and a self-loop reads
/// `[a, a]`. Keys are visited in `adjacency` order, making the reported cycle
/// deterministic for a given input.
pub fn find_keyed_cycle<K: Eq + Hash + Clone>(adjacency: &[(K, Vec<K>)]) -> Option<Vec<K>> {
    let edges: HashMap<&K, &[K]> = adjacency.iter().map(|(k, ks)| (k, ks.as_slice())).collect();
    let mut marks: HashMap<&K, Mark> = HashMap::new();

    for (root, _) in adjacency {
        if marks.contains_key(root) {
            continue;
        }
        // `stack` holds (key, next-edge-index) frames; `path` mirrors the active
        // frames so a back-edge can be turned into the cycle it closes.
        let mut stack: Vec<(&K, usize)> = vec![(root, 0)];
        let mut path: Vec<&K> = vec![root];
        marks.insert(root, Mark::Visiting);

        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            let Some(target) = edges.get(node).and_then(|targets| targets.get(*next)) else {
                marks.insert(node, Mark::Done);
                path.pop();
                stack.pop();
                continue;
            };
            *next += 1;

            if !edges.contains_key(target) {
                continue;
            }
            match marks.get(target) {
                Some(Mark::Visiting) => {
                    let entry = path.iter().position(|k| *k == target).unwrap_or(0);
                    return Some(
                        path[entry..]
                            .iter()
                            .map(|k| (*k).clone())
                            .chain(std::iter::once(target.clone()))
                            .collect(),
                    );
                }
                Some(Mark::Done) => {}
                None => {
                    marks.insert(target, Mark::Visiting);
                    path.push(target);
                    stack.push((target, 0));
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adjacency<'k>(pairs: &[(&'k str, &[&'k str])]) -> Vec<(&'k str, Vec<&'k str>)> {
        pairs
            .iter()
            .map(|(key, targets)| (*key, targets.to_vec()))
            .collect()
    }

    #[test]
    fn test_find_keyed_cycle_acyclic_returns_none() {
        let graph = adjacency(&[("a", &["b", "c"]), ("b", &["c"]), ("c", &[])]);
        assert_eq!(find_keyed_cycle(&graph), None);
    }

    #[test]
    fn test_find_keyed_cycle_empty_graph_returns_none() {
        let graph: Vec<(&str, Vec<&str>)> = vec![];
        assert_eq!(find_keyed_cycle(&graph), None);
    }

    #[test]
    fn test_find_keyed_cycle_reports_closed_path() {
        let graph = adjacency(&[("a", &["b"]), ("b", &["c"]), ("c", &["a"])]);
        assert_eq!(find_keyed_cycle(&graph), Some(vec!["a", "b", "c", "a"]));
    }

    #[test]
    fn test_find_keyed_cycle_reports_self_loop() {
        let graph = adjacency(&[("a", &["a"])]);
        assert_eq!(find_keyed_cycle(&graph), Some(vec!["a", "a"]));
    }

    #[test]
    fn test_find_keyed_cycle_excludes_acyclic_prefix_from_path() {
        // `a` reaches the cycle b → c → b but is not part of it.
        let graph = adjacency(&[("a", &["b"]), ("b", &["c"]), ("c", &["b"])]);
        assert_eq!(find_keyed_cycle(&graph), Some(vec!["b", "c", "b"]));
    }

    #[test]
    fn test_find_keyed_cycle_ignores_edges_to_unknown_keys() {
        let graph = adjacency(&[("a", &["missing"]), ("b", &["a", "gone"])]);
        assert_eq!(find_keyed_cycle(&graph), None);
    }

    #[test]
    fn test_find_keyed_cycle_finds_cycle_in_later_component() {
        let graph = adjacency(&[("a", &["b"]), ("b", &[]), ("c", &["d"]), ("d", &["c"])]);
        assert_eq!(find_keyed_cycle(&graph), Some(vec!["c", "d", "c"]));
    }

    #[test]
    fn test_find_keyed_cycle_revisits_shared_node_once() {
        // Diamond: a → b → d, a → c → d. `d` is reached twice, no cycle.
        let graph = adjacency(&[("a", &["b", "c"]), ("b", &["d"]), ("c", &["d"]), ("d", &[])]);
        assert_eq!(find_keyed_cycle(&graph), None);
    }

    #[test]
    fn test_find_keyed_cycle_works_over_index_keys() {
        let graph: Vec<(usize, Vec<usize>)> = vec![(0, vec![1]), (1, vec![2]), (2, vec![0])];
        assert_eq!(find_keyed_cycle(&graph), Some(vec![0, 1, 2, 0]));
    }
}
