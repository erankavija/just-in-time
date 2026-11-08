//! Dependency graph operations and validation.
//!
//! Provides DAG enforcement, cycle detection, and graph traversal operations.
//!
//! The graph module provides a generic `DependencyGraph<T>` that works with any
//! type implementing the `GraphNode` trait. This allows the same DAG algorithms
//! to be used for issues, tasks, packages, or any other dependency relationships.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use thiserror::Error;

pub mod hierarchy;
pub mod keyed;

pub use keyed::{find_keyed_cycle, keyed_topological_order};

/// Which way a traversal follows dependency edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Outgoing edges: the nodes a node depends on.
    Dependencies,
    /// Incoming edges: the nodes that depend on a node.
    Dependents,
}

/// One node of a depth-bounded unfolding of the graph ([`DependencyGraph::expand`]).
///
/// The unfolding is a tree: a node reachable by several paths appears once per
/// path, each occurrence carrying its own `level` (1 for a direct neighbour of
/// the expansion root).
#[derive(Debug)]
pub struct Expansion<'a, T> {
    /// The node this occurrence stands for.
    pub node: &'a T,
    /// Distance from the expansion root, counting from 1.
    pub level: u32,
    /// Occurrences of this node's own neighbours, empty at the depth bound.
    pub children: Vec<Expansion<'a, T>>,
}

/// Trait for types that can participate in a dependency graph
///
/// Types implementing this trait can be used with `DependencyGraph` to enforce
/// DAG properties, detect cycles, and perform graph traversals.
pub trait GraphNode {
    /// Identity preserved across graph queries.
    type Id: Clone + Eq + Hash + Ord;

    /// Unique identifier for this node
    fn id(&self) -> &Self::Id;

    /// IDs of nodes this node depends on
    fn dependencies(&self) -> impl Iterator<Item = &Self::Id> + '_;
}

/// Errors that can occur during graph operations
#[derive(Debug, Error, PartialEq)]
pub enum GraphError<Id> {
    /// A cycle was detected in the dependency graph
    #[error("Cycle detected: adding dependency would create a cycle")]
    CycleDetected,
    /// Identity of a reference to an absent node.
    #[error("Node not found: {id}")]
    NodeNotFound { id: Id },
}

/// The depth-first walk under [`DependencyGraph::traverse`], over any
/// adjacency: `neighbours` yields an id's neighbours in adjacency order,
/// `resolve` the node an id names, and `admits` whether a resolved node is
/// reached. Each reached node is returned once, in preorder, without `start`;
/// an id that resolves to no node or to one `admits` rejects is neither
/// reached nor walked through. `max_depth` counts edges from `start`, 0 being
/// unlimited.
pub fn traverse_with<'n, Id: Eq + Hash + ?Sized, N>(
    start: &'n Id,
    max_depth: u32,
    neighbours: impl Fn(&Id) -> Vec<&'n Id>,
    resolve: impl Fn(&Id) -> Option<N>,
    admits: impl Fn(&N) -> bool,
) -> Vec<N> {
    let frontier = |id: &Id, depth: u32| -> Vec<(&'n Id, u32)> {
        neighbours(id)
            .into_iter()
            .rev()
            .map(|id| (id, depth))
            .collect()
    };

    // Reverse-push so the stack pops neighbours in adjacency order; seeding
    // `visited` with `start` keeps the root out of its own result.
    let mut stack = frontier(start, 1);
    let mut visited: HashSet<&Id> = HashSet::from([start]);
    let mut reached = Vec::new();

    while let Some((id, depth)) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Some(node) = resolve(id).filter(|node| admits(node)) else {
            continue;
        };
        reached.push(node);
        if max_depth == 0 || depth < max_depth {
            stack.extend(frontier(id, depth + 1));
        }
    }

    reached
}

/// Generic dependency graph with cycle detection and traversal
///
/// Provides DAG enforcement and graph operations for any type implementing `GraphNode`.
/// All methods are pure functions that do not modify the graph structure.
pub struct DependencyGraph<'a, T: GraphNode> {
    nodes: HashMap<T::Id, &'a T>,
    /// Reverse-dependency index: each depended-on id's dependents, deduplicated
    /// and sorted by id so a dependents traversal is deterministic. Built once in
    /// [`new`](Self::new); every dependents query reads it.
    dependents: HashMap<&'a T::Id, Vec<&'a T>>,
}

#[cfg(test)]
thread_local! {
    static CONSTRUCTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The number of [`DependencyGraph::new`] calls this thread has made.
#[cfg(test)]
pub(crate) fn constructions_on_this_thread() -> usize {
    CONSTRUCTIONS.with(std::cell::Cell::get)
}

impl<'a, T: GraphNode> DependencyGraph<'a, T> {
    /// Create a new dependency graph from a list of nodes, building its
    /// reverse-dependency index in O(V + E log E).
    pub fn new(nodes: &[&'a T]) -> Self {
        #[cfg(test)]
        CONSTRUCTIONS.with(|count| count.set(count.get() + 1));
        let nodes_map: HashMap<T::Id, &'a T> = nodes
            .iter()
            .map(|node| (node.id().clone(), *node))
            .collect();
        let mut dependents: HashMap<&'a T::Id, Vec<&'a T>> = HashMap::new();
        nodes_map.values().for_each(|node| {
            node.dependencies()
                .for_each(|dep| dependents.entry(dep).or_default().push(*node))
        });
        dependents.values_mut().for_each(|list| {
            list.sort_unstable_by(|a, b| a.id().cmp(b.id()));
            list.dedup_by(|a, b| a.id() == b.id());
        });

        Self {
            nodes: nodes_map,
            dependents,
        }
    }

    /// The ids `id` reaches by one edge in `direction`: dependency-declaration
    /// order for [`Direction::Dependencies`], id order for
    /// [`Direction::Dependents`].
    fn neighbors(&self, id: &T::Id, direction: Direction) -> Vec<&'a T::Id> {
        match direction {
            Direction::Dependencies => self
                .nodes
                .get(id)
                .map(|node| node.dependencies().collect())
                .unwrap_or_default(),
            Direction::Dependents => self.dependents_of(id).map(GraphNode::id).collect(),
        }
    }

    /// The direct dependents of `id`, read from the reverse index.
    fn dependents_of(&self, id: &T::Id) -> impl Iterator<Item = &'a T> + '_ {
        self.dependents.get(id).into_iter().flatten().copied()
    }

    /// Walk the graph from `start` in `direction`, stopping at `max_depth`.
    ///
    /// Returns each reachable node once, in depth-first preorder, excluding
    /// `start` itself. `max_depth` counts edges from `start`: 1 yields the
    /// immediate neighbours, 0 is unlimited. Edges pointing at absent nodes are
    /// skipped.
    ///
    /// At `max_depth = 0` and [`Direction::Dependencies`] this returns the same
    /// nodes as [`get_transitive_dependencies`](Self::get_transitive_dependencies).
    pub fn traverse(&self, start: &T::Id, direction: Direction, max_depth: u32) -> Vec<&'a T> {
        traverse_with(
            start,
            max_depth,
            |id| self.neighbors(id, direction),
            |id| self.nodes.get(id).copied(),
            |_| true,
        )
    }

    /// Unfold the graph from `start` in `direction` into a tree, stopping at
    /// `max_depth`.
    ///
    /// Returns the occurrences of `start`'s neighbours; `start` itself is not in
    /// the result. Unlike [`traverse`](Self::traverse), a node reached by several
    /// paths is unfolded once per path, which is what a dependency tree renders.
    /// `max_depth` counts edges from `start`, 0 being unlimited.
    pub fn expand(
        &self,
        start: &T::Id,
        direction: Direction,
        max_depth: u32,
    ) -> Vec<Expansion<'a, T>> {
        self.expand_from(start, 1, max_depth, direction)
    }

    /// Unfold the neighbours of `id` at `level`, recursing until `max_depth`.
    ///
    /// Terminates on any acyclic graph: every step descends one level, and a
    /// non-zero `max_depth` bounds the descent regardless.
    fn expand_from(
        &self,
        id: &T::Id,
        level: u32,
        max_depth: u32,
        direction: Direction,
    ) -> Vec<Expansion<'a, T>> {
        self.neighbors(id, direction)
            .into_iter()
            .filter_map(|neighbor| self.nodes.get(neighbor).copied())
            .map(|node| Expansion {
                node,
                level,
                children: match max_depth == 0 || level < max_depth {
                    true => self.expand_from(node.id(), level + 1, max_depth, direction),
                    false => Vec::new(),
                },
            })
            .collect()
    }

    /// Validate that adding a dependency would not create a cycle
    pub fn validate_add_dependency(
        &self,
        from_id: &T::Id,
        to_id: &T::Id,
    ) -> Result<(), GraphError<T::Id>> {
        if !self.nodes.contains_key(from_id) {
            return Err(GraphError::NodeNotFound {
                id: from_id.clone(),
            });
        }
        if !self.nodes.contains_key(to_id) {
            return Err(GraphError::NodeNotFound { id: to_id.clone() });
        }

        // Check if adding this dependency would create a cycle
        // We simulate adding the edge and check for cycles
        if self.would_create_cycle(from_id, to_id) {
            return Err(GraphError::CycleDetected);
        }

        Ok(())
    }

    fn would_create_cycle(&self, from: &T::Id, to: &T::Id) -> bool {
        // Adding edge from -> to creates a cycle if there's already a path to -> from
        // In other words, if 'from' is reachable from 'to'
        self.is_reachable(to, from)
    }

    fn is_reachable(&self, start: &T::Id, target: &T::Id) -> bool {
        let mut visited = HashSet::new();
        let mut stack = vec![start];

        while let Some(current) = stack.pop() {
            if current == target {
                return true;
            }

            if visited.contains(current) {
                continue;
            }
            visited.insert(current);

            if let Some(node) = self.nodes.get(current) {
                for dep in node.dependencies() {
                    stack.push(dep);
                }
            }
        }

        false
    }

    /// Every node, sorted by id.
    pub fn nodes_by_id(&self) -> Vec<&'a T> {
        let mut nodes: Vec<&'a T> = self.nodes.values().copied().collect();
        nodes.sort_unstable_by(|a, b| a.id().cmp(b.id()));
        nodes
    }

    /// Get all root nodes (nodes with no dependencies)
    pub fn get_roots(&self) -> Vec<&'a T> {
        self.nodes
            .values()
            .filter(|node| node.dependencies().next().is_none())
            .copied()
            .collect()
    }

    /// Every node that directly depends on `node_id`, sorted by id.
    pub fn get_dependents(&self, node_id: &T::Id) -> Vec<&'a T> {
        self.dependents_of(node_id).collect()
    }

    /// Every node that transitively depends on `node_id`.
    pub fn get_transitive_dependents(&self, node_id: &T::Id) -> Vec<&'a T> {
        let mut reached: HashSet<&T::Id> = HashSet::new();
        let mut result = Vec::new();
        let mut stack = vec![node_id];

        while let Some(current) = stack.pop() {
            for dependent in self.dependents_of(current) {
                if reached.insert(dependent.id()) {
                    result.push(dependent);
                    stack.push(dependent.id());
                }
            }
        }
        result
    }

    /// Get all nodes the given node transitively DEPENDS ON.
    ///
    /// Follows outgoing dependency edges from `node_id` to their closure,
    /// excluding `node_id` itself. The mirror of
    /// [`get_transitive_dependents`](Self::get_transitive_dependents), it answers
    /// "what does this node (transitively) require?" — used to assemble an issue's
    /// dependency neighborhood for transition-time graph-rule evaluation.
    pub fn get_transitive_dependencies(&self, node_id: &T::Id) -> Vec<&'a T> {
        let mut result: HashSet<&T::Id> = HashSet::new();
        let mut stack = vec![node_id];
        let mut visited: HashSet<&T::Id> = HashSet::new();

        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(node) = self.nodes.get(current) {
                for dep in node.dependencies() {
                    result.insert(dep);
                    stack.push(dep);
                }
            }
        }

        result
            .into_iter()
            .filter(|id| *id != node_id)
            .filter_map(|id| self.nodes.get(id).copied())
            .collect()
    }

    /// Every node with neither dependencies nor dependents, in O(V + E).
    pub fn get_isolated_nodes(&self) -> Vec<&'a T> {
        self.nodes
            .values()
            .filter(|node| {
                node.dependencies().next().is_none() && !self.dependents.contains_key(node.id())
            })
            .copied()
            .collect()
    }

    /// Validate that the graph is a DAG; the only error is [`GraphError::CycleDetected`].
    pub fn validate_dag(&self) -> Result<(), GraphError<T::Id>> {
        // Check for cycles using DFS
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for id in self.nodes.keys() {
            if !visited.contains(id) && self.has_cycle_dfs(id, &mut visited, &mut rec_stack) {
                return Err(GraphError::CycleDetected);
            }
        }

        Ok(())
    }

    fn has_cycle_dfs(
        &self,
        node: &T::Id,
        visited: &mut HashSet<T::Id>,
        rec_stack: &mut HashSet<T::Id>,
    ) -> bool {
        visited.insert(node.clone());
        rec_stack.insert(node.clone());

        if let Some(graph_node) = self.nodes.get(node) {
            for dep in graph_node.dependencies() {
                if !visited.contains(dep) {
                    if self.has_cycle_dfs(dep, visited, rec_stack) {
                        return true;
                    }
                } else if rec_stack.contains(dep) {
                    return true;
                }
            }
        }

        rec_stack.remove(node);
        false
    }

    /// Check if edge from→to is transitive (redundant).
    ///
    /// An edge is transitive if there exists a path from→...→to
    /// through other edges (i.e., path length > 1).
    pub fn is_transitive(&self, from: &T::Id, to: &T::Id) -> bool {
        // Check if there's a path from→to excluding the direct edge
        self.has_path_excluding_direct(from, to)
    }

    /// Check if path exists from start to target, excluding the direct edge.
    fn has_path_excluding_direct(&self, start: &T::Id, target: &T::Id) -> bool {
        let mut visited = HashSet::new();
        let mut stack = vec![start];

        while let Some(current) = stack.pop() {
            if current == target && current != start {
                return true;
            }

            if visited.contains(current) {
                continue;
            }
            visited.insert(current);

            if let Some(node) = self.nodes.get(current) {
                for dep in node.dependencies() {
                    // Skip the direct edge from start to target
                    if current == start && dep == target {
                        continue;
                    }
                    stack.push(dep);
                }
            }
        }

        false
    }

    /// Compute transitive reduction for a single node's dependencies.
    ///
    /// Returns the minimal set of dependencies that preserves reachability.
    /// An edge is kept only if it's not reachable through other edges.
    /// References to absent nodes are skipped.
    pub fn compute_transitive_reduction(&self, node_id: &T::Id) -> Vec<&'a T> {
        let Some(node) = self.nodes.get(node_id) else {
            return Vec::new();
        };

        let deps: Vec<&T::Id> = node.dependencies().collect();
        let mut kept = HashSet::new();

        // Keep only dependencies that are NOT reachable through other dependencies
        deps.iter()
            .copied()
            .filter(|dep| {
                // Check if this dep is reachable through other deps
                let other_deps: Vec<&T::Id> = deps.iter().filter(|d| *d != dep).copied().collect();

                // If dep is reachable from any other dep, it's redundant
                !other_deps.iter().any(|other| self.is_reachable(other, dep))
            })
            .filter(|dep| kept.insert(*dep))
            .filter_map(|dep| self.nodes.get(dep).copied())
            .collect()
    }

    /// Find shortest path between two nodes (excluding direct edge).
    ///
    /// Returns the shortest path that doesn't use the direct edge from `from` to `to`.
    /// Useful for reporting alternative paths when detecting redundant dependencies.
    ///
    /// Returns nodes along the path, or an empty vector if no path exists.
    /// References to absent nodes are skipped.
    pub fn find_shortest_path(&self, from: &T::Id, to: &T::Id) -> Vec<&'a T> {
        use std::collections::VecDeque;

        let Some(start) = self.nodes.get(from).copied() else {
            return Vec::new();
        };
        let mut queue = VecDeque::new();
        let mut visited = HashMap::new();

        queue.push_back((start, vec![start]));

        while let Some((node, path)) = queue.pop_front() {
            let current = node.id();
            if current == to && path.len() > 1 {
                return path;
            }

            if visited.contains_key(current) {
                continue;
            }
            visited.insert(current, ());

            for dep in node.dependencies() {
                // Skip direct edge from start
                if current == from && dep == to {
                    continue;
                }

                if let Some(dependency) = self.nodes.get(dep).copied() {
                    let mut new_path = path.clone();
                    new_path.push(dependency);
                    queue.push_back((dependency, new_path));
                }
            }
        }

        vec![]
    }

    /// Find every direct edge that violates transitive reduction.
    ///
    /// Returns each edge `(from, to)` that is redundant because `to` is already
    /// reachable from `from` through another of `from`'s dependencies. This is
    /// exactly the property `jit validate` enforces at read time (built on
    /// [`compute_transitive_reduction`](Self::compute_transitive_reduction)),
    /// exposed as a pure query so the write path can reject a redundant edge
    /// before persisting it instead of surfacing the violation at a later
    /// `jit validate`. The result is sorted for deterministic reporting.
    /// References to absent nodes are skipped.
    pub fn find_redundant_edges(&self) -> Vec<(&'a T, &'a T)> {
        let mut redundant = Vec::new();
        for node in self.nodes.values() {
            let id = node.id();
            let reduced: HashSet<&T::Id> = self
                .compute_transitive_reduction(id)
                .into_iter()
                .map(GraphNode::id)
                .collect();
            for dep in node.dependencies() {
                if !reduced.contains(dep) {
                    if let Some(dependency) = self.nodes.get(dep).copied() {
                        redundant.push((*node, dependency));
                    }
                }
            }
        }
        redundant.sort_unstable_by(|(a, b), (c, d)| (a.id(), b.id()).cmp(&(c.id(), d.id())));
        redundant
    }
}

impl<'a, T: GraphNode> Expansion<'a, T> {
    /// How many occurrences each node id has across `forest`.
    ///
    /// A count above 1 marks a node shared between paths, the diamonds a
    /// dependency tree flags.
    pub fn occurrences(forest: &[Self]) -> HashMap<&'a T::Id, usize> {
        let mut counts: HashMap<&'a T::Id, usize> = HashMap::new();
        let mut stack: Vec<&Self> = forest.iter().collect();
        while let Some(occurrence) = stack.pop() {
            *counts.entry(occurrence.node.id()).or_insert(0) += 1;
            stack.extend(occurrence.children.iter());
        }
        counts
    }

    /// Rebuild this occurrence bottom-up in the caller's shape.
    ///
    /// `build` receives a node, its level, and its already-built children. It
    /// lets a command project an expansion into a view type without walking the
    /// tree itself.
    pub fn fold<N>(&self, build: &impl Fn(&'a T, u32, Vec<N>) -> N) -> N {
        let children = self
            .children
            .iter()
            .map(|child| child.fold(build))
            .collect();
        build(self.node, self.level, children)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Dummy node type for testing generic graph functionality
    #[derive(Debug, Clone)]
    struct TestNode {
        id: String,
        deps: Vec<String>,
    }

    impl TestNode {
        fn new(id: &str, deps: Vec<&str>) -> Self {
            Self {
                id: id.to_string(),
                deps: deps.iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    impl GraphNode for TestNode {
        type Id = String;

        fn id(&self) -> &String {
            &self.id
        }

        fn dependencies(&self) -> impl Iterator<Item = &String> + '_ {
            self.deps.iter()
        }
    }

    // Tests with generic TestNode type
    #[test]
    fn test_generic_graph_with_test_nodes() {
        let node1 = TestNode::new("A", vec![]);
        let node2 = TestNode::new("B", vec!["A"]);
        let node3 = TestNode::new("C", vec!["B"]);

        let nodes = vec![&node1, &node2, &node3];
        let graph = DependencyGraph::new(&nodes);

        // C -> B -> A chain exists, so adding C -> A is OK (redundant but valid)
        assert!(graph
            .validate_add_dependency(&"C".to_string(), &"A".to_string())
            .is_ok());
        // But A -> C would create a cycle: A -> C -> B -> A
        assert_eq!(
            graph.validate_add_dependency(&"A".to_string(), &"C".to_string()),
            Err(GraphError::CycleDetected)
        );
    }

    #[test]
    fn test_generic_graph_roots() {
        let node1 = TestNode::new("root1", vec![]);
        let node2 = TestNode::new("dep", vec!["root1"]);
        let node3 = TestNode::new("root2", vec![]);

        let nodes = vec![&node1, &node2, &node3];
        let graph = DependencyGraph::new(&nodes);

        let roots = graph.get_roots();
        assert_eq!(roots.len(), 2);
        assert!(roots.iter().any(|n| n.id() == "root1"));
        assert!(roots.iter().any(|n| n.id() == "root2"));
    }

    #[test]
    fn test_generic_graph_cycle_detection() {
        let mut node1 = TestNode::new("A", vec![]);
        let mut node2 = TestNode::new("B", vec![]);
        node1.deps.push("B".to_string());
        node2.deps.push("A".to_string());

        let nodes = vec![&node1, &node2];
        let graph = DependencyGraph::new(&nodes);

        assert_eq!(graph.validate_dag(), Err(GraphError::CycleDetected));
    }

    // Tests with Issue type (existing functionality)
    #[test]
    fn test_validate_add_dependency_success() {
        let issue1 = crate::test_utils::fixture_issue("Issue 1".to_string(), "Desc".to_string());
        let issue2 = crate::test_utils::fixture_issue("Issue 2".to_string(), "Desc".to_string());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        assert!(graph
            .validate_add_dependency(&issue1.id, &issue2.id)
            .is_ok());
    }

    #[test]
    fn test_validate_add_dependency_with_nonexistent_issue() {
        let issue1 = crate::test_utils::fixture_issue("Issue 1".to_string(), "Desc".to_string());

        let issues = vec![&issue1];
        let graph = DependencyGraph::new(&issues);

        let missing = crate::test_utils::issue_id(1);
        let result = graph.validate_add_dependency(&issue1.id, &missing);
        assert_eq!(
            result,
            Err(GraphError::NodeNotFound {
                id: missing.clone()
            })
        );
        assert_eq!(
            graph.validate_add_dependency(&missing, &issue1.id),
            Err(GraphError::NodeNotFound { id: missing })
        );
    }

    #[test]
    fn test_validate_add_dependency_detects_direct_cycle() {
        let mut issue1 =
            crate::test_utils::fixture_issue("Issue 1".to_string(), "Desc".to_string());
        let issue2 = crate::test_utils::fixture_issue("Issue 2".to_string(), "Desc".to_string());

        // issue1 depends on issue2
        issue1.dependencies.push(issue2.id.clone());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        // Trying to make issue2 depend on issue1 would create a cycle
        let result = graph.validate_add_dependency(&issue2.id, &issue1.id);
        assert_eq!(result, Err(GraphError::CycleDetected));
    }

    #[test]
    fn test_validate_add_dependency_detects_indirect_cycle() {
        let mut issue1 =
            crate::test_utils::fixture_issue("Issue 1".to_string(), "Desc".to_string());
        let mut issue2 =
            crate::test_utils::fixture_issue("Issue 2".to_string(), "Desc".to_string());
        let issue3 = crate::test_utils::fixture_issue("Issue 3".to_string(), "Desc".to_string());

        // issue1 -> issue2 -> issue3
        issue1.dependencies.push(issue2.id.clone());
        issue2.dependencies.push(issue3.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        // Trying to make issue3 depend on issue1 would create a cycle
        let result = graph.validate_add_dependency(&issue3.id, &issue1.id);
        assert_eq!(result, Err(GraphError::CycleDetected));
    }

    #[test]
    fn test_get_roots_returns_issues_with_no_dependencies() {
        let issue1 = crate::test_utils::fixture_issue("Root 1".to_string(), "Desc".to_string());
        let mut issue2 =
            crate::test_utils::fixture_issue("Dependent".to_string(), "Desc".to_string());
        let issue3 = crate::test_utils::fixture_issue("Root 2".to_string(), "Desc".to_string());

        issue2.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        let roots = graph.get_roots();
        assert_eq!(roots.len(), 2);
        assert!(roots.iter().any(|i| i.id == issue1.id));
        assert!(roots.iter().any(|i| i.id == issue3.id));
    }

    #[test]
    fn test_validate_dag_success_for_valid_graph() {
        let issue1 = crate::test_utils::fixture_issue("Issue 1".to_string(), "Desc".to_string());
        let mut issue2 =
            crate::test_utils::fixture_issue("Issue 2".to_string(), "Desc".to_string());
        let mut issue3 =
            crate::test_utils::fixture_issue("Issue 3".to_string(), "Desc".to_string());

        issue2.dependencies.push(issue1.id.clone());
        issue3.dependencies.push(issue2.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        assert!(graph.validate_dag().is_ok());
    }

    #[test]
    fn test_validate_dag_detects_cycle() {
        let mut issue1 =
            crate::test_utils::fixture_issue("Issue 1".to_string(), "Desc".to_string());
        let mut issue2 =
            crate::test_utils::fixture_issue("Issue 2".to_string(), "Desc".to_string());

        issue1.dependencies.push(issue2.id.clone());
        issue2.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        assert_eq!(graph.validate_dag(), Err(GraphError::CycleDetected));
    }

    // Tests for transitive reduction

    #[test]
    fn test_is_transitive_simple_chain() {
        // A→B→C, test if A→C is transitive
        let a = TestNode::new("A", vec!["B"]);
        let b = TestNode::new("B", vec!["C"]);
        let c = TestNode::new("C", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c]);

        // A→C is transitive (reachable via A→B→C)
        assert!(graph.is_transitive(&"A".to_string(), &"C".to_string()));
        // A→B is NOT transitive (direct edge)
        assert!(!graph.is_transitive(&"A".to_string(), &"B".to_string()));
        // B→C is NOT transitive (direct edge)
        assert!(!graph.is_transitive(&"B".to_string(), &"C".to_string()));
    }

    #[test]
    fn test_is_transitive_diamond() {
        // Diamond: A→B, A→C, B→C
        // A→C is transitive via A→B→C
        let a = TestNode::new("A", vec!["B", "C"]);
        let b = TestNode::new("B", vec!["C"]);
        let c = TestNode::new("C", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c]);

        // A→C is transitive (reachable via A→B→C)
        assert!(graph.is_transitive(&"A".to_string(), &"C".to_string()));
        // A→B is NOT transitive
        assert!(!graph.is_transitive(&"A".to_string(), &"B".to_string()));
        // B→C is NOT transitive
        assert!(!graph.is_transitive(&"B".to_string(), &"C".to_string()));
    }

    #[test]
    fn test_is_transitive_no_path() {
        // A→B, C→D (disconnected)
        let a = TestNode::new("A", vec!["B"]);
        let b = TestNode::new("B", vec![]);
        let c = TestNode::new("C", vec!["D"]);
        let d = TestNode::new("D", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        // A→D has no path at all
        assert!(!graph.is_transitive(&"A".to_string(), &"D".to_string()));
    }

    #[test]
    fn test_is_transitive_complex_graph() {
        // A→B, A→C, A→D, B→D, C→D
        // A→D is transitive via both A→B→D and A→C→D
        let a = TestNode::new("A", vec!["B", "C", "D"]);
        let b = TestNode::new("B", vec!["D"]);
        let c = TestNode::new("C", vec!["D"]);
        let d = TestNode::new("D", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        // A→D is transitive (reachable via A→B→D and A→C→D)
        assert!(graph.is_transitive(&"A".to_string(), &"D".to_string()));
        // Direct edges are not transitive
        assert!(!graph.is_transitive(&"A".to_string(), &"B".to_string()));
        assert!(!graph.is_transitive(&"A".to_string(), &"C".to_string()));
        assert!(!graph.is_transitive(&"B".to_string(), &"D".to_string()));
        assert!(!graph.is_transitive(&"C".to_string(), &"D".to_string()));
    }

    #[test]
    fn test_is_transitive_long_chain() {
        // A→B→C→D→E, test A→E
        let a = TestNode::new("A", vec!["B"]);
        let b = TestNode::new("B", vec!["C"]);
        let c = TestNode::new("C", vec!["D"]);
        let d = TestNode::new("D", vec!["E"]);
        let e = TestNode::new("E", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c, &d, &e]);

        // A→E is transitive (long path)
        assert!(graph.is_transitive(&"A".to_string(), &"E".to_string()));
        // All direct edges are not transitive
        assert!(!graph.is_transitive(&"A".to_string(), &"B".to_string()));
        assert!(!graph.is_transitive(&"B".to_string(), &"C".to_string()));
        assert!(!graph.is_transitive(&"C".to_string(), &"D".to_string()));
        assert!(!graph.is_transitive(&"D".to_string(), &"E".to_string()));
    }

    #[test]
    fn test_find_shortest_path_returns_alternative_nodes_in_order() {
        let a = TestNode::new("A", vec!["C", "B"]);
        let b = TestNode::new("B", vec!["C"]);
        let c = TestNode::new("C", vec![]);
        let graph = DependencyGraph::new(&[&a, &b, &c]);

        let path = graph.find_shortest_path(&"A".to_string(), &"C".to_string());

        assert_eq!(
            path.iter()
                .map(|node| node.id().as_str())
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
    }

    #[test]
    fn test_graph_queries_skip_absent_nodes() {
        let a = TestNode::new("A", vec!["B", "missing"]);
        let b = TestNode::new("B", vec!["missing"]);
        let graph = DependencyGraph::new(&[&a, &b]);

        assert_eq!(
            graph
                .compute_transitive_reduction(&"A".to_string())
                .iter()
                .map(|node| node.id().as_str())
                .collect::<Vec<_>>(),
            ["B"]
        );
        assert!(graph
            .compute_transitive_reduction(&"missing".to_string())
            .is_empty());
        assert!(graph
            .find_shortest_path(&"A".to_string(), &"missing".to_string())
            .is_empty());
        assert!(graph
            .find_shortest_path(&"missing".to_string(), &"A".to_string())
            .is_empty());
        assert!(graph.find_redundant_edges().is_empty());
    }

    #[test]
    fn test_compute_transitive_reduction_simple() {
        // A→B, A→C, B→C
        // Should keep only A→B and B→C
        let a = TestNode::new("A", vec!["B", "C"]);
        let b = TestNode::new("B", vec!["C"]);
        let c = TestNode::new("C", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c]);
        let reduced_deps = graph
            .compute_transitive_reduction(&"A".to_string())
            .into_iter()
            .map(|node| node.id().as_str())
            .collect::<HashSet<_>>();

        // A should only depend on B (not C)
        assert_eq!(reduced_deps.len(), 1);
        assert!(reduced_deps.contains("B"));
        assert!(!reduced_deps.contains("C"));
    }

    #[test]
    fn test_compute_transitive_reduction_no_redundancy() {
        // A→B, A→C (parallel, no transitive path)
        let a = TestNode::new("A", vec!["B", "C"]);
        let b = TestNode::new("B", vec![]);
        let c = TestNode::new("C", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c]);
        let reduced_deps = graph
            .compute_transitive_reduction(&"A".to_string())
            .into_iter()
            .map(|node| node.id().as_str())
            .collect::<HashSet<_>>();

        // Both dependencies should remain
        assert_eq!(reduced_deps.len(), 2);
        assert!(reduced_deps.contains("B"));
        assert!(reduced_deps.contains("C"));
    }

    #[test]
    fn test_compute_transitive_reduction_complex() {
        // A→B, A→C, A→D, B→D, C→D
        // Should keep only A→B and A→C (remove A→D)
        let a = TestNode::new("A", vec!["B", "C", "D"]);
        let b = TestNode::new("B", vec!["D"]);
        let c = TestNode::new("C", vec!["D"]);
        let d = TestNode::new("D", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);
        let reduced_deps = graph
            .compute_transitive_reduction(&"A".to_string())
            .into_iter()
            .map(|node| node.id().as_str())
            .collect::<HashSet<_>>();

        // A should depend on B and C, but not D
        assert_eq!(reduced_deps.len(), 2);
        assert!(reduced_deps.contains("B"));
        assert!(reduced_deps.contains("C"));
        assert!(!reduced_deps.contains("D"));
    }

    // Tests for depth-bounded traversal and expansion

    fn ids(nodes: &[&TestNode]) -> Vec<String> {
        nodes.iter().map(|node| node.id().to_string()).collect()
    }

    fn sorted_ids(nodes: &[&TestNode]) -> Vec<String> {
        let mut ids = ids(nodes);
        ids.sort();
        ids
    }

    /// A → B → C → D chain plus a shortcut A → C, exercising depth bounds and
    /// the shared node C.
    fn chain_with_shortcut() -> (TestNode, TestNode, TestNode, TestNode) {
        (
            TestNode::new("A", vec!["B", "C"]),
            TestNode::new("B", vec!["C"]),
            TestNode::new("C", vec!["D"]),
            TestNode::new("D", vec![]),
        )
    }

    #[test]
    fn test_traverse_dependencies_depth_one_returns_immediate_neighbors() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        assert_eq!(
            ids(&graph.traverse(&"A".to_string(), Direction::Dependencies, 1)),
            vec!["B", "C"]
        );
    }

    #[test]
    fn test_traverse_dependencies_respects_depth_bound() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        // Depth 2 reaches C (via B or the shortcut) but stops before D.
        assert_eq!(
            sorted_ids(&graph.traverse(&"A".to_string(), Direction::Dependencies, 2)),
            vec!["B", "C"]
        );
        assert_eq!(
            sorted_ids(&graph.traverse(&"A".to_string(), Direction::Dependencies, 3)),
            vec!["B", "C", "D"]
        );
    }

    #[test]
    fn test_traverse_dependencies_visits_each_node_once() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        // C is reachable as A→C and A→B→C; it is returned once.
        assert_eq!(
            sorted_ids(&graph.traverse(&"A".to_string(), Direction::Dependencies, 0)),
            vec!["B", "C", "D"]
        );
    }

    #[test]
    fn test_traverse_excludes_start_node() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        assert!(graph
            .traverse(&"A".to_string(), Direction::Dependencies, 0)
            .iter()
            .all(|node| node.id() != "A"));
        assert!(graph
            .traverse(&"D".to_string(), Direction::Dependents, 0)
            .iter()
            .all(|node| node.id() != "D"));
    }

    #[test]
    fn test_traverse_dependents_walks_incoming_edges() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        assert_eq!(
            ids(&graph.traverse(&"C".to_string(), Direction::Dependents, 1)),
            vec!["A", "B"]
        );
        assert_eq!(
            sorted_ids(&graph.traverse(&"D".to_string(), Direction::Dependents, 0)),
            vec!["A", "B", "C"]
        );
        assert_eq!(
            ids(&graph.traverse(&"D".to_string(), Direction::Dependents, 1)),
            vec!["C"]
        );
    }

    #[test]
    fn test_traverse_skips_edges_to_absent_nodes() {
        let a = TestNode::new("A", vec!["missing", "B"]);
        let b = TestNode::new("B", vec![]);
        let graph = DependencyGraph::new(&[&a, &b]);

        assert_eq!(
            ids(&graph.traverse(&"A".to_string(), Direction::Dependencies, 0)),
            vec!["B"]
        );
    }

    #[test]
    fn test_traverse_unknown_start_returns_nothing() {
        let a = TestNode::new("A", vec![]);
        let graph = DependencyGraph::new(&[&a]);

        assert!(graph
            .traverse(&"nonexistent".to_string(), Direction::Dependencies, 0)
            .is_empty());
    }

    // REQ-4: unlimited forward traversal agrees with the transitive-dependencies
    // primitive on a hand-built diamond, and (below) on arbitrary DAGs.
    #[test]
    fn test_traverse_unlimited_matches_transitive_dependencies() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        let mut expected: Vec<String> = graph
            .get_transitive_dependencies(&"A".to_string())
            .iter()
            .map(|node| node.id().to_string())
            .collect();
        expected.sort();

        assert_eq!(
            sorted_ids(&graph.traverse(&"A".to_string(), Direction::Dependencies, 0)),
            expected
        );
    }

    #[test]
    fn test_expand_unfolds_shared_node_once_per_path() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        let forest = graph.expand(&"A".to_string(), Direction::Dependencies, 0);
        // A's neighbours, in declaration order.
        assert_eq!(
            forest.iter().map(|e| e.node.id()).collect::<Vec<_>>(),
            vec!["B", "C"]
        );
        assert!(forest.iter().all(|e| e.level == 1));
        // B's subtree re-unfolds C, so C occurs twice overall (and D with it).
        let counts = Expansion::occurrences(&forest);
        assert_eq!(counts.get(&"C".to_string()), Some(&2));
        assert_eq!(counts.get(&"D".to_string()), Some(&2));
        assert_eq!(counts.get(&"B".to_string()), Some(&1));
    }

    #[test]
    fn test_expand_stops_at_depth_bound() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        let forest = graph.expand(&"A".to_string(), Direction::Dependencies, 1);
        assert_eq!(forest.len(), 2);
        assert!(forest.iter().all(|e| e.children.is_empty()));

        let forest = graph.expand(&"A".to_string(), Direction::Dependencies, 2);
        let b_node = forest.iter().find(|e| e.node.id() == "B").unwrap();
        assert_eq!(b_node.children.len(), 1);
        assert_eq!(b_node.children[0].node.id(), "C");
        assert_eq!(b_node.children[0].level, 2);
        assert!(b_node.children[0].children.is_empty());
    }

    #[test]
    fn test_expand_fold_rebuilds_tree_bottom_up() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        let render = |node: &TestNode, level: u32, children: Vec<String>| {
            format!("{}@{}{}", node.id(), level, children.join(""))
        };
        let rendered: Vec<String> = graph
            .expand(&"A".to_string(), Direction::Dependencies, 0)
            .iter()
            .map(|expansion| expansion.fold(&render))
            .collect();

        assert_eq!(rendered, vec!["B@1C@2D@3", "C@1D@2"]);
    }

    /// Random DAG: node `i` may only depend on nodes `< i`, so `edges[i]` is the
    /// mask of admissible predecessors, keeping the graph acyclic by construction.
    fn dag_from_masks(masks: &[u32]) -> Vec<TestNode> {
        masks
            .iter()
            .enumerate()
            .map(|(index, mask)| {
                let deps: Vec<String> = (0..index)
                    .filter(|earlier| mask & (1 << earlier) != 0)
                    .map(|earlier| earlier.to_string())
                    .collect();
                TestNode {
                    id: index.to_string(),
                    deps,
                }
            })
            .collect()
    }

    proptest::proptest! {
        /// Every reverse-index query equals its definition over the input edges:
        /// direct dependents list the node, transitive dependents reach it, and
        /// an isolated node has no edge at either end.
        #[test]
        fn test_dependents_index_queries_equal_their_definitions_over_arbitrary_dags(
            masks in proptest::collection::vec(0u32..128, 0..8),
        ) {
            let nodes = dag_from_masks(&masks);
            let refs: Vec<&TestNode> = nodes.iter().collect();
            let graph = DependencyGraph::new(&refs);
            let reaches = |from: &TestNode, target: &str| {
                let mut stack = vec![from];
                let mut seen = HashSet::new();
                while let Some(node) = stack.pop() {
                    if !seen.insert(node.id.as_str()) {
                        continue;
                    }
                    if node.deps.iter().any(|dep| dep == target) {
                        return true;
                    }
                    stack.extend(nodes.iter().filter(|n| node.deps.contains(&n.id)));
                }
                false
            };
            let defined = |keep: &dyn Fn(&TestNode) -> bool| -> Vec<String> {
                nodes.iter().filter(|n| keep(n)).map(|n| n.id.clone()).collect()
            };

            for node in &nodes {
                proptest::prop_assert_eq!(
                    sorted_ids(&graph.get_dependents(node.id())),
                    defined(&|n| n.deps.contains(&node.id))
                );
                let transitive = defined(&|n| reaches(n, &node.id));
                proptest::prop_assert_eq!(
                    sorted_ids(&graph.get_transitive_dependents(node.id())),
                    transitive.clone()
                );
                proptest::prop_assert_eq!(
                    sorted_ids(&graph.traverse(node.id(), Direction::Dependents, 0)),
                    transitive
                );
            }
            proptest::prop_assert_eq!(
                sorted_ids(&graph.get_isolated_nodes()),
                defined(&|n| n.deps.is_empty() && !nodes.iter().any(|m| m.deps.contains(&n.id)))
            );
        }

        #[test]
        fn test_graph_queries_borrow_nodes_from_the_input_set(
            masks in proptest::collection::vec(0u32..128, 1..8),
        ) {
            let nodes = dag_from_masks(&masks);
            let refs: Vec<&TestNode> = nodes.iter().collect();
            let graph = DependencyGraph::new(&refs);
            let belongs_to_input = |node: &TestNode| refs.iter().any(|input| std::ptr::eq(*input, node));

            for node in &nodes {
                for dependency in graph.compute_transitive_reduction(node.id()) {
                    proptest::prop_assert!(belongs_to_input(dependency));
                }
                for target in &nodes {
                    for step in graph.find_shortest_path(node.id(), target.id()) {
                        proptest::prop_assert!(belongs_to_input(step));
                    }
                }
            }
            for (from, to) in graph.find_redundant_edges() {
                proptest::prop_assert!(belongs_to_input(from));
                proptest::prop_assert!(belongs_to_input(to));
            }
        }

        /// REQ-4: on any DAG, unlimited forward traversal reaches exactly the
        /// nodes `get_transitive_dependencies` reports.
        #[test]
        fn prop_traverse_unlimited_matches_transitive_dependencies(
            masks in proptest::collection::vec(0u32..128, 1..8),
        ) {
            let nodes = dag_from_masks(&masks);
            let refs: Vec<&TestNode> = nodes.iter().collect();
            let graph = DependencyGraph::new(&refs);

            for node in &nodes {
                let traversed = sorted_ids(&graph.traverse(node.id(), Direction::Dependencies, 0));
                let mut expected: Vec<String> = graph
                    .get_transitive_dependencies(node.id())
                    .iter()
                    .map(|n| n.id().to_string())
                    .collect();
                expected.sort();
                proptest::prop_assert_eq!(traversed, expected);
            }
        }

        /// A DAG built with only backward edges has no cycle, whatever the mask.
        #[test]
        fn prop_find_keyed_cycle_reports_none_for_dags(
            masks in proptest::collection::vec(0u32..128, 1..8),
        ) {
            let nodes = dag_from_masks(&masks);
            let adjacency: Vec<(String, Vec<String>)> = nodes
                .into_iter()
                .map(|node| (node.id, node.deps))
                .collect();
            proptest::prop_assert_eq!(find_keyed_cycle(&adjacency), None);
        }
    }
}
