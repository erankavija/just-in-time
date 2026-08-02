//! Acyclicity and ordering over symbolic keyed graphs.
//!
//! [`DependencyGraph`](super::DependencyGraph) needs nodes that carry a real id.
//! Callers that reason about a graph before it exists — batch creation over
//! symbolic keys, breakdown over child indices, template roles, profile-package
//! ids — hold nothing but an adjacency list. [`find_keyed_cycle`] and
//! [`keyed_topological_order`] serve those: both are generic over any
//! `(K, Vec<K>)` adjacency, the first reporting the offending path and the
//! second ordering the keys that path would make unorderable.

use std::collections::{HashMap, HashSet};
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

/// Order the keys of a keyed adjacency list so each key follows every key it
/// points at, or report the cycle that makes such an order impossible
/// (`@/inv/dag-acyclic`).
///
/// `adjacency` means what it means for [`find_keyed_cycle`]: each key is paired
/// with the keys it points at, an edge to a key absent from `adjacency` is
/// ignored, and a key that repeats takes its edges from its last entry. Read as
/// "depends on", the answer is dependency-first — every key comes after
/// everything it depends on, directly or transitively — which is the order a
/// caller applies them in.
///
/// The order holds each key of `adjacency` exactly once. Keys are visited in
/// `adjacency` order, so both the order and the reported cycle are deterministic
/// for a given input, and `Err` carries the cycle exactly as
/// [`find_keyed_cycle`] reports it: a closed path whose first key repeats as its
/// last.
pub fn keyed_topological_order<K: Eq + Hash + Clone>(
    adjacency: &[(K, Vec<K>)],
) -> Result<Vec<K>, Vec<K>> {
    if let Some(cycle) = find_keyed_cycle(adjacency) {
        return Err(cycle);
    }
    let edges: HashMap<&K, &[K]> = adjacency.iter().map(|(k, ks)| (k, ks.as_slice())).collect();
    let mut visited: HashSet<&K> = HashSet::with_capacity(edges.len());
    let mut order: Vec<K> = Vec::with_capacity(edges.len());

    for (root, _) in adjacency {
        if !visited.insert(root) {
            continue;
        }
        // The graph is acyclic here, so a key leaves the stack only once every
        // key it points at has already left it — which is what makes the pop
        // order the answer.
        let mut stack: Vec<(&K, usize)> = vec![(root, 0)];
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            let Some(target) = edges.get(node).and_then(|targets| targets.get(*next)) else {
                order.push((*node).clone());
                stack.pop();
                continue;
            };
            *next += 1;
            if edges.contains_key(target) && visited.insert(target) {
                stack.push((target, 0));
            }
        }
    }

    Ok(order)
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

    /// The position of `key` in `order`, or a panic naming what was ordered.
    fn position<K: Eq + std::fmt::Debug>(order: &[K], key: &K) -> usize {
        order
            .iter()
            .position(|candidate| candidate == key)
            .unwrap_or_else(|| panic!("{key:?} is missing from {order:?}"))
    }

    /// Assert the ordering contract over `graph`: every key once, and each key
    /// after every key of the graph it points at.
    fn assert_targets_precede_sources<'k>(graph: &[(&'k str, Vec<&'k str>)], order: &[&'k str]) {
        let keys: std::collections::BTreeSet<&str> = graph.iter().map(|(key, _)| *key).collect();
        assert_eq!(
            order
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            keys,
            "the order must hold every key and nothing else"
        );
        assert_eq!(order.len(), keys.len(), "a key must be ordered once");
        for (source, targets) in graph {
            for target in targets.iter().filter(|target| keys.contains(*target)) {
                assert!(
                    position(order, target) < position(order, source),
                    "{target} must precede {source} in {order:?}"
                );
            }
        }
    }

    #[test]
    fn test_keyed_topological_order_places_a_chain_deepest_first() {
        let graph = adjacency(&[("a", &["b"]), ("b", &["c"]), ("c", &[])]);

        let order = keyed_topological_order(&graph).expect("an acyclic chain orders");

        assert_targets_precede_sources(&graph, &order);
        assert_eq!(order, vec!["c", "b", "a"]);
    }

    #[test]
    fn test_keyed_topological_order_orders_a_diamond_with_the_shared_key_once() {
        // a → b → d, a → c → d: `d` is reached twice and ordered once, before
        // both of the keys that reach it.
        let graph = adjacency(&[("a", &["b", "c"]), ("b", &["d"]), ("c", &["d"]), ("d", &[])]);

        let order = keyed_topological_order(&graph).expect("a diamond orders");

        assert_targets_precede_sources(&graph, &order);
        assert_eq!(order.iter().filter(|key| **key == "d").count(), 1);
    }

    #[test]
    fn test_keyed_topological_order_reports_the_cycle_it_cannot_order() {
        let graph = adjacency(&[("a", &["b"]), ("b", &["c"]), ("c", &["a"])]);

        let cycle = keyed_topological_order(&graph).expect_err("a cycle has no order");

        // The reported path is the cycle itself, closed, so a caller can name it
        // without re-deriving it.
        assert_eq!(cycle, vec!["a", "b", "c", "a"]);
        assert_eq!(Some(cycle), find_keyed_cycle(&graph));
    }

    #[test]
    fn test_keyed_topological_order_reports_a_cycle_no_key_of_it_roots() {
        // `a` reaches the cycle b → c → b without being part of it; the order is
        // still impossible, and what is reported is the cycle rather than the
        // path that found it.
        let graph = adjacency(&[("a", &["b"]), ("b", &["c"]), ("c", &["b"])]);

        assert_eq!(
            keyed_topological_order(&graph).expect_err("a reachable cycle has no order"),
            vec!["b", "c", "b"]
        );
    }

    #[test]
    fn test_keyed_topological_order_ignores_edges_to_unknown_keys() {
        // An edge to a key the adjacency does not carry constrains nothing, so
        // it neither orders nor removes anything.
        let graph = adjacency(&[("a", &["missing", "b"]), ("b", &["gone"])]);

        let order = keyed_topological_order(&graph).expect("unknown targets do not block ordering");

        assert_targets_precede_sources(&graph, &order);
    }

    #[test]
    fn test_keyed_topological_order_carries_every_component() {
        let graph = adjacency(&[("a", &["b"]), ("b", &[]), ("c", &["d"]), ("d", &[])]);

        let order = keyed_topological_order(&graph).expect("disconnected components order");

        assert_targets_precede_sources(&graph, &order);
    }

    #[test]
    fn test_keyed_topological_order_orders_an_empty_graph() {
        let graph: Vec<(&str, Vec<&str>)> = vec![];

        assert_eq!(keyed_topological_order(&graph), Ok(Vec::new()));
    }

    #[test]
    fn test_keyed_topological_order_takes_a_repeated_keys_last_edges() {
        // The repeated key is ordered once, and the edges that constrain it are
        // its last entry's — the same rule cycle detection applies.
        let graph = adjacency(&[("a", &[]), ("b", &["a"]), ("b", &[])]);

        let order = keyed_topological_order(&graph).expect("a repeated key orders");

        assert_eq!(order, vec!["a", "b"]);
        assert_eq!(
            keyed_topological_order(&adjacency(&[("a", &["b"]), ("b", &["a"]), ("b", &[])]))
                .expect("the last entry drops the closing edge"),
            vec!["b", "a"]
        );
    }

    #[test]
    fn test_keyed_topological_order_works_over_index_keys() {
        let graph: Vec<(usize, Vec<usize>)> = vec![(0, vec![1]), (1, vec![2]), (2, vec![])];

        assert_eq!(keyed_topological_order(&graph), Ok(vec![2, 1, 0]));
    }

    /// One adjacency over `0..masks.len()`, where bit `j` of `masks[i]` is the
    /// edge `i → j`. Every edge direction is reachable, so the generated graphs
    /// include self-loops and cycles.
    fn adjacency_from_masks(masks: &[u32]) -> Vec<(usize, Vec<usize>)> {
        masks
            .iter()
            .enumerate()
            .map(|(index, mask)| {
                let targets = (0..masks.len())
                    .filter(|target| mask & (1 << target) != 0)
                    .collect();
                (index, targets)
            })
            .collect()
    }

    proptest::proptest! {
        /// On any keyed graph, an order exists exactly when no cycle does, and
        /// the refusal is the cycle detector's own answer.
        #[test]
        fn prop_keyed_topological_order_refuses_exactly_the_cyclic_graphs(
            masks in proptest::collection::vec(0u32..64, 1..7),
        ) {
            let graph = adjacency_from_masks(&masks);

            match keyed_topological_order(&graph) {
                Ok(_) => proptest::prop_assert_eq!(find_keyed_cycle(&graph), None),
                Err(cycle) => proptest::prop_assert_eq!(Some(cycle), find_keyed_cycle(&graph)),
            }
        }

        /// On any acyclic keyed graph, the order carries every key once and
        /// places each key after every key it points at.
        #[test]
        fn prop_keyed_topological_order_places_every_target_before_its_source(
            masks in proptest::collection::vec(0u32..64, 1..7),
        ) {
            // Masked down to backward edges only, so the generated graph is a
            // DAG whatever the drawn masks were.
            let graph: Vec<(usize, Vec<usize>)> = adjacency_from_masks(&masks)
                .into_iter()
                .map(|(index, targets)| {
                    (index, targets.into_iter().filter(|target| *target < index).collect())
                })
                .collect();

            let order = keyed_topological_order(&graph).expect("a backward-edged graph is acyclic");

            proptest::prop_assert_eq!(order.len(), graph.len());
            proptest::prop_assert_eq!(
                order.iter().copied().collect::<std::collections::BTreeSet<_>>().len(),
                graph.len()
            );
            for (source, targets) in &graph {
                for target in targets {
                    proptest::prop_assert!(
                        position(&order, target) < position(&order, source),
                        "{} must precede {} in {:?}", target, source, order
                    );
                }
            }
        }

        /// The order is a function of the input alone: two runs agree.
        #[test]
        fn prop_keyed_topological_order_is_deterministic(
            masks in proptest::collection::vec(0u32..64, 1..7),
        ) {
            let graph = adjacency_from_masks(&masks);

            proptest::prop_assert_eq!(
                keyed_topological_order(&graph),
                keyed_topological_order(&graph)
            );
        }
    }
}
