//! Dependency graph operations and validation.
//!
//! Provides DAG enforcement, cycle detection, and graph traversal operations.
//!
//! The graph module provides a generic `DependencyGraph<T>` that works with any
//! type implementing the `GraphNode` trait. This allows the same DAG algorithms
//! to be used for issues, tasks, packages, or any other dependency relationships.

use std::collections::{HashMap, HashSet};
use thiserror::Error;

pub mod hierarchy;
pub mod keyed;

pub use keyed::find_keyed_cycle;

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
    /// Unique identifier for this node
    fn id(&self) -> &str;

    /// IDs of nodes this node depends on
    fn dependencies(&self) -> &[String];
}

/// Errors that can occur during graph operations
#[derive(Debug, Error, PartialEq)]
pub enum GraphError {
    /// A cycle was detected in the dependency graph
    #[error("Cycle detected: adding dependency would create a cycle")]
    CycleDetected,
    /// Referenced node does not exist
    #[error("Node not found: {id}")]
    NodeNotFound { id: String },
}

/// Generic dependency graph with cycle detection and traversal
///
/// Provides DAG enforcement and graph operations for any type implementing `GraphNode`.
/// All methods are pure functions that do not modify the graph structure.
pub struct DependencyGraph<'a, T: GraphNode> {
    nodes: HashMap<String, &'a T>,
}

impl<'a, T: GraphNode> DependencyGraph<'a, T> {
    /// Create a new dependency graph from a list of nodes
    pub fn new(nodes: &[&'a T]) -> Self {
        let nodes_map = nodes
            .iter()
            .map(|node| (node.id().to_string(), *node))
            .collect();

        Self { nodes: nodes_map }
    }

    /// Adjacency in `direction`, indexed by node id.
    ///
    /// Building it once amortizes the reverse lookup that
    /// [`get_dependents`](Self::get_dependents) pays per call. Neighbour lists
    /// keep dependency-declaration order for [`Direction::Dependencies`] and are
    /// sorted by id for [`Direction::Dependents`], so a traversal is
    /// deterministic in either direction.
    fn neighbors(&self, direction: Direction) -> HashMap<&'a str, Vec<&'a str>> {
        match direction {
            Direction::Dependencies => self
                .nodes
                .values()
                .map(|node| {
                    (
                        node.id(),
                        node.dependencies().iter().map(String::as_str).collect(),
                    )
                })
                .collect(),
            Direction::Dependents => {
                let mut index: HashMap<&'a str, Vec<&'a str>> = self
                    .nodes
                    .values()
                    .map(|node| (node.id(), Vec::new()))
                    .collect();
                for node in self.nodes.values() {
                    for dep in node.dependencies() {
                        if let Some(dependents) = index.get_mut(dep.as_str()) {
                            dependents.push(node.id());
                        }
                    }
                }
                index
                    .values_mut()
                    .for_each(|dependents| dependents.sort_unstable());
                index
            }
        }
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
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::Issue;
    /// use jit::graph::{DependencyGraph, Direction};
    ///
    /// let a = Issue::new("A".into(), "".into());
    /// let mut b = Issue::new("B".into(), "".into());
    /// let mut c = Issue::new("C".into(), "".into());
    /// b.dependencies.push(a.id.clone());
    /// c.dependencies.push(b.id.clone());
    ///
    /// let graph = DependencyGraph::new(&[&a, &b, &c]);
    /// let immediate = graph.traverse(&c.id, Direction::Dependencies, 1);
    /// assert_eq!(immediate.len(), 1);
    /// assert_eq!(immediate[0].id, b.id);
    /// assert_eq!(graph.traverse(&c.id, Direction::Dependencies, 0).len(), 2);
    /// ```
    pub fn traverse(&self, start: &str, direction: Direction, max_depth: u32) -> Vec<&'a T> {
        let neighbors = self.neighbors(direction);
        let frontier = |id: &str, depth: u32| -> Vec<(&'a str, u32)> {
            neighbors
                .get(id)
                .into_iter()
                .flat_map(|ids| ids.iter().rev().map(move |id| (*id, depth)))
                .collect()
        };

        // Reverse-push so the stack pops neighbours in adjacency order; seeding
        // `visited` with `start` keeps the root out of its own result.
        let mut stack = frontier(start, 1);
        let mut visited: HashSet<&str> = HashSet::from([start]);
        let mut reached = Vec::new();

        while let Some((id, depth)) = stack.pop() {
            if !visited.insert(id) {
                continue;
            }
            let Some(node) = self.nodes.get(id).copied() else {
                continue;
            };
            reached.push(node);
            if max_depth == 0 || depth < max_depth {
                stack.extend(frontier(id, depth + 1));
            }
        }

        reached
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
        start: &str,
        direction: Direction,
        max_depth: u32,
    ) -> Vec<Expansion<'a, T>> {
        self.expand_from(start, 1, max_depth, &self.neighbors(direction))
    }

    /// Unfold the neighbours of `id` at `level`, recursing until `max_depth`.
    ///
    /// Terminates on any acyclic graph: every step descends one level, and a
    /// non-zero `max_depth` bounds the descent regardless.
    fn expand_from(
        &self,
        id: &str,
        level: u32,
        max_depth: u32,
        neighbors: &HashMap<&'a str, Vec<&'a str>>,
    ) -> Vec<Expansion<'a, T>> {
        neighbors
            .get(id)
            .into_iter()
            .flatten()
            .filter_map(|neighbor| self.nodes.get(*neighbor).copied())
            .map(|node| Expansion {
                node,
                level,
                children: match max_depth == 0 || level < max_depth {
                    true => self.expand_from(node.id(), level + 1, max_depth, neighbors),
                    false => Vec::new(),
                },
            })
            .collect()
    }

    /// Validate that adding a dependency would not create a cycle
    pub fn validate_add_dependency(&self, from_id: &str, to_id: &str) -> Result<(), GraphError> {
        if !self.nodes.contains_key(from_id) {
            return Err(GraphError::NodeNotFound {
                id: from_id.to_string(),
            });
        }
        if !self.nodes.contains_key(to_id) {
            return Err(GraphError::NodeNotFound {
                id: to_id.to_string(),
            });
        }

        // Check if adding this dependency would create a cycle
        // We simulate adding the edge and check for cycles
        if self.would_create_cycle(from_id, to_id) {
            return Err(GraphError::CycleDetected);
        }

        Ok(())
    }

    fn would_create_cycle(&self, from: &str, to: &str) -> bool {
        // Adding edge from -> to creates a cycle if there's already a path to -> from
        // In other words, if 'from' is reachable from 'to'
        self.is_reachable(to, from)
    }

    fn is_reachable(&self, start: &str, target: &str) -> bool {
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
                    stack.push(dep.as_str());
                }
            }
        }

        false
    }

    /// Get all root nodes (nodes with no dependencies)
    pub fn get_roots(&self) -> Vec<&'a T> {
        self.nodes
            .values()
            .filter(|node| node.dependencies().is_empty())
            .copied()
            .collect()
    }

    /// Get all nodes that directly depend on the given node
    pub fn get_dependents(&self, node_id: &str) -> Vec<&'a T> {
        self.nodes
            .values()
            .filter(|node| node.dependencies().contains(&node_id.to_string()))
            .copied()
            .collect()
    }

    /// Get all nodes that transitively depend on the given node
    pub fn get_transitive_dependents(&self, node_id: &str) -> Vec<&'a T> {
        let mut result = HashSet::new();
        let mut stack = vec![node_id];
        let mut visited = HashSet::new();

        while let Some(current) = stack.pop() {
            if visited.contains(current) {
                continue;
            }
            visited.insert(current);

            let dependents = self.get_dependents(current);
            for dependent in dependents {
                result.insert(dependent.id());
                stack.push(dependent.id());
            }
        }

        result
            .into_iter()
            .filter_map(|id| self.nodes.get(id).copied())
            .collect()
    }

    /// Get all nodes the given node transitively DEPENDS ON.
    ///
    /// Follows outgoing dependency edges from `node_id` to their closure,
    /// excluding `node_id` itself. The mirror of
    /// [`get_transitive_dependents`](Self::get_transitive_dependents), it answers
    /// "what does this node (transitively) require?" — used to assemble an issue's
    /// dependency neighborhood for transition-time graph-rule evaluation.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::Issue;
    /// use jit::graph::DependencyGraph;
    ///
    /// let a = Issue::new("A".into(), "".into());
    /// let mut b = Issue::new("B".into(), "".into());
    /// let mut c = Issue::new("C".into(), "".into());
    /// b.dependencies.push(a.id.clone()); // B depends on A
    /// c.dependencies.push(b.id.clone()); // C depends on B (and transitively A)
    ///
    /// let graph = DependencyGraph::new(&[&a, &b, &c]);
    /// let mut deps: Vec<&str> = graph
    ///     .get_transitive_dependencies(&c.id)
    ///     .iter()
    ///     .map(|i| i.id.as_str())
    ///     .collect();
    /// deps.sort();
    /// let mut expected = vec![a.id.as_str(), b.id.as_str()];
    /// expected.sort();
    /// assert_eq!(deps, expected);
    /// ```
    pub fn get_transitive_dependencies(&self, node_id: &str) -> Vec<&'a T> {
        let mut result: HashSet<&str> = HashSet::new();
        let mut stack = vec![node_id];
        let mut visited: HashSet<&str> = HashSet::new();

        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(node) = self.nodes.get(current) {
                for dep in node.dependencies() {
                    result.insert(dep.as_str());
                    stack.push(dep.as_str());
                }
            }
        }

        result
            .into_iter()
            .filter(|id| *id != node_id)
            .filter_map(|id| self.nodes.get(id).copied())
            .collect()
    }

    /// Get all isolated nodes (nodes with no dependencies and no dependents)
    ///
    /// An isolated node is one that has no incoming or outgoing edges in the
    /// dependency graph. These nodes are not connected to any other node.
    pub fn get_isolated_nodes(&self) -> Vec<&'a T> {
        self.nodes
            .values()
            .filter(|node| {
                // No dependencies (no outgoing edges)
                let has_no_dependencies = node.dependencies().is_empty();
                // No dependents (no incoming edges)
                let has_no_dependents = self.get_dependents(node.id()).is_empty();
                has_no_dependencies && has_no_dependents
            })
            .copied()
            .collect()
    }

    /// Validate that the graph is a DAG (no cycles)
    pub fn validate_dag(&self) -> Result<(), GraphError> {
        // Check for cycles using DFS
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for id in self.nodes.keys() {
            if !visited.contains(id.as_str())
                && self.has_cycle_dfs(id, &mut visited, &mut rec_stack)
            {
                return Err(GraphError::CycleDetected);
            }
        }

        Ok(())
    }

    fn has_cycle_dfs(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
    ) -> bool {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());

        if let Some(graph_node) = self.nodes.get(node) {
            for dep in graph_node.dependencies() {
                if !visited.contains(dep.as_str()) {
                    if self.has_cycle_dfs(dep, visited, rec_stack) {
                        return true;
                    }
                } else if rec_stack.contains(dep.as_str()) {
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
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::Issue;
    /// use jit::graph::DependencyGraph;
    ///
    /// let a = Issue::new("A".into(), "".into());
    /// let mut b = Issue::new("B".into(), "".into());
    /// let mut c = Issue::new("C".into(), "".into());
    /// b.dependencies.push(a.id.clone());
    /// c.dependencies.push(b.id.clone());
    /// c.dependencies.push(a.id.clone()); // Redundant!
    ///
    /// let graph = DependencyGraph::new(&[&a, &b, &c]);
    /// assert!(graph.is_transitive(&c.id, &a.id)); // C→A is transitive via C→B→A
    /// ```
    pub fn is_transitive(&self, from: &str, to: &str) -> bool {
        // Check if there's a path from→to excluding the direct edge
        self.has_path_excluding_direct(from, to)
    }

    /// Check if path exists from start to target, excluding the direct edge.
    fn has_path_excluding_direct(&self, start: &str, target: &str) -> bool {
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
                    stack.push(dep.as_str());
                }
            }
        }

        false
    }

    /// Compute transitive reduction for a single node's dependencies.
    ///
    /// Returns the minimal set of dependencies that preserves reachability.
    /// An edge is kept only if it's not reachable through other edges.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::Issue;
    /// use jit::graph::DependencyGraph;
    ///
    /// let a = Issue::new("A".into(), "".into());
    /// let mut b = Issue::new("B".into(), "".into());
    /// let mut c = Issue::new("C".into(), "".into());
    /// b.dependencies.push(a.id.clone());
    /// c.dependencies.push(b.id.clone());
    /// c.dependencies.push(a.id.clone()); // Redundant
    ///
    /// let graph = DependencyGraph::new(&[&a, &b, &c]);
    /// let reduced = graph.compute_transitive_reduction(&c.id);
    ///
    /// assert_eq!(reduced.len(), 1);
    /// assert!(reduced.contains(&b.id)); // Only keep C→B
    /// ```
    pub fn compute_transitive_reduction(&self, node_id: &str) -> HashSet<String> {
        let Some(node) = self.nodes.get(node_id) else {
            return HashSet::new();
        };

        let deps = node.dependencies();
        if deps.is_empty() {
            return HashSet::new();
        }

        // Keep only dependencies that are NOT reachable through other dependencies
        deps.iter()
            .filter(|dep| {
                // Check if this dep is reachable through other deps
                let other_deps: Vec<&str> = deps
                    .iter()
                    .filter(|d| d != dep)
                    .map(String::as_str)
                    .collect();

                // If dep is reachable from any other dep, it's redundant
                !other_deps.iter().any(|other| self.is_reachable(other, dep))
            })
            .cloned()
            .collect()
    }

    /// Find shortest path between two nodes (excluding direct edge).
    ///
    /// Returns the shortest path that doesn't use the direct edge from `from` to `to`.
    /// Useful for reporting alternative paths when detecting redundant dependencies.
    ///
    /// Returns a vector of node IDs representing the path, or empty vector if no path exists.
    pub fn find_shortest_path(&self, from: &str, to: &str) -> Vec<String> {
        use std::collections::VecDeque;

        let mut queue = VecDeque::new();
        let mut visited = HashMap::new();

        queue.push_back((from, vec![from.to_string()]));

        while let Some((current, path)) = queue.pop_front() {
            if current == to && path.len() > 1 {
                return path;
            }

            if visited.contains_key(current) {
                continue;
            }
            visited.insert(current, ());

            if let Some(node) = self.nodes.get(current) {
                for dep in node.dependencies() {
                    // Skip direct edge from start
                    if current == from && dep == to {
                        continue;
                    }

                    let mut new_path = path.clone();
                    new_path.push(dep.clone());
                    queue.push_back((dep.as_str(), new_path));
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
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::domain::Issue;
    /// use jit::graph::DependencyGraph;
    ///
    /// let a = Issue::new("A".into(), "".into());
    /// let mut b = Issue::new("B".into(), "".into());
    /// let mut c = Issue::new("C".into(), "".into());
    /// b.dependencies.push(a.id.clone());
    /// c.dependencies.push(b.id.clone());
    /// c.dependencies.push(a.id.clone()); // redundant: C→A already via C→B→A
    ///
    /// let graph = DependencyGraph::new(&[&a, &b, &c]);
    /// let redundant = graph.find_redundant_edges();
    /// assert_eq!(redundant, vec![(c.id.clone(), a.id.clone())]);
    /// ```
    pub fn find_redundant_edges(&self) -> Vec<(String, String)> {
        let mut redundant: Vec<(String, String)> = Vec::new();
        for node in self.nodes.values() {
            let id = node.id();
            let reduced = self.compute_transitive_reduction(id);
            for dep in node.dependencies() {
                if !reduced.contains(dep) {
                    redundant.push((id.to_string(), dep.clone()));
                }
            }
        }
        redundant.sort();
        redundant
    }
}

impl<'a, T: GraphNode> Expansion<'a, T> {
    /// How many occurrences each node id has across `forest`.
    ///
    /// A count above 1 marks a node shared between paths, the diamonds a
    /// dependency tree flags.
    pub fn occurrences(forest: &[Self]) -> HashMap<&'a str, usize> {
        let mut counts: HashMap<&'a str, usize> = HashMap::new();
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
    use crate::domain::Issue;

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
        fn id(&self) -> &str {
            &self.id
        }

        fn dependencies(&self) -> &[String] {
            &self.deps
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
        assert!(graph.validate_add_dependency("C", "A").is_ok());
        // But A -> C would create a cycle: A -> C -> B -> A
        assert_eq!(
            graph.validate_add_dependency("A", "C"),
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
    fn test_generic_graph_dependents() {
        let node1 = TestNode::new("base", vec![]);
        let node2 = TestNode::new("dep1", vec!["base"]);
        let node3 = TestNode::new("dep2", vec!["base"]);

        let nodes = vec![&node1, &node2, &node3];
        let graph = DependencyGraph::new(&nodes);

        let dependents = graph.get_dependents("base");
        assert_eq!(dependents.len(), 2);
        assert!(dependents.iter().any(|n| n.id() == "dep1"));
        assert!(dependents.iter().any(|n| n.id() == "dep2"));
    }

    #[test]
    fn test_generic_graph_transitive_dependents() {
        let node1 = TestNode::new("root", vec![]);
        let node2 = TestNode::new("level1", vec!["root"]);
        let node3 = TestNode::new("level2", vec!["level1"]);

        let nodes = vec![&node1, &node2, &node3];
        let graph = DependencyGraph::new(&nodes);

        let transitive = graph.get_transitive_dependents("root");
        assert_eq!(transitive.len(), 2);
        assert!(transitive.iter().any(|n| n.id() == "level1"));
        assert!(transitive.iter().any(|n| n.id() == "level2"));
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
        let issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());
        let issue2 = Issue::new("Issue 2".to_string(), "Desc".to_string());

        let issues = vec![&issue1, &issue2];
        let graph = DependencyGraph::new(&issues);

        assert!(graph
            .validate_add_dependency(&issue1.id, &issue2.id)
            .is_ok());
    }

    #[test]
    fn test_validate_add_dependency_with_nonexistent_issue() {
        let issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());

        let issues = vec![&issue1];
        let graph = DependencyGraph::new(&issues);

        let result = graph.validate_add_dependency(&issue1.id, "nonexistent");
        assert_eq!(
            result,
            Err(GraphError::NodeNotFound {
                id: "nonexistent".to_string()
            })
        );
    }

    #[test]
    fn test_validate_add_dependency_detects_direct_cycle() {
        let mut issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());
        let issue2 = Issue::new("Issue 2".to_string(), "Desc".to_string());

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
        let mut issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Issue 2".to_string(), "Desc".to_string());
        let issue3 = Issue::new("Issue 3".to_string(), "Desc".to_string());

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
        let issue1 = Issue::new("Root 1".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Dependent".to_string(), "Desc".to_string());
        let issue3 = Issue::new("Root 2".to_string(), "Desc".to_string());

        issue2.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        let roots = graph.get_roots();
        assert_eq!(roots.len(), 2);
        assert!(roots.iter().any(|i| i.id == issue1.id));
        assert!(roots.iter().any(|i| i.id == issue3.id));
    }

    #[test]
    fn test_get_dependents_returns_direct_dependents() {
        let issue1 = Issue::new("Dependency".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Dependent 1".to_string(), "Desc".to_string());
        let mut issue3 = Issue::new("Dependent 2".to_string(), "Desc".to_string());

        issue2.dependencies.push(issue1.id.clone());
        issue3.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        let dependents = graph.get_dependents(&issue1.id);
        assert_eq!(dependents.len(), 2);
        assert!(dependents.iter().any(|i| i.id == issue2.id));
        assert!(dependents.iter().any(|i| i.id == issue3.id));
    }

    #[test]
    fn test_get_transitive_dependents() {
        let issue1 = Issue::new("Root".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Level 1".to_string(), "Desc".to_string());
        let mut issue3 = Issue::new("Level 2".to_string(), "Desc".to_string());

        issue2.dependencies.push(issue1.id.clone());
        issue3.dependencies.push(issue2.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        let transitive = graph.get_transitive_dependents(&issue1.id);
        assert_eq!(transitive.len(), 2);
        assert!(transitive.iter().any(|i| i.id == issue2.id));
        assert!(transitive.iter().any(|i| i.id == issue3.id));
    }

    #[test]
    fn test_validate_dag_success_for_valid_graph() {
        let issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Issue 2".to_string(), "Desc".to_string());
        let mut issue3 = Issue::new("Issue 3".to_string(), "Desc".to_string());

        issue2.dependencies.push(issue1.id.clone());
        issue3.dependencies.push(issue2.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        assert!(graph.validate_dag().is_ok());
    }

    #[test]
    fn test_validate_dag_detects_cycle() {
        let mut issue1 = Issue::new("Issue 1".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Issue 2".to_string(), "Desc".to_string());

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
        assert!(graph.is_transitive("A", "C"));
        // A→B is NOT transitive (direct edge)
        assert!(!graph.is_transitive("A", "B"));
        // B→C is NOT transitive (direct edge)
        assert!(!graph.is_transitive("B", "C"));
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
        assert!(graph.is_transitive("A", "C"));
        // A→B is NOT transitive
        assert!(!graph.is_transitive("A", "B"));
        // B→C is NOT transitive
        assert!(!graph.is_transitive("B", "C"));
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
        assert!(!graph.is_transitive("A", "D"));
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
        assert!(graph.is_transitive("A", "D"));
        // Direct edges are not transitive
        assert!(!graph.is_transitive("A", "B"));
        assert!(!graph.is_transitive("A", "C"));
        assert!(!graph.is_transitive("B", "D"));
        assert!(!graph.is_transitive("C", "D"));
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
        assert!(graph.is_transitive("A", "E"));
        // All direct edges are not transitive
        assert!(!graph.is_transitive("A", "B"));
        assert!(!graph.is_transitive("B", "C"));
        assert!(!graph.is_transitive("C", "D"));
        assert!(!graph.is_transitive("D", "E"));
    }

    #[test]
    fn test_compute_transitive_reduction_simple() {
        // A→B, A→C, B→C
        // Should keep only A→B and B→C
        let a = TestNode::new("A", vec!["B", "C"]);
        let b = TestNode::new("B", vec!["C"]);
        let c = TestNode::new("C", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c]);
        let reduced_deps = graph.compute_transitive_reduction("A");

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
        let reduced_deps = graph.compute_transitive_reduction("A");

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
        let reduced_deps = graph.compute_transitive_reduction("A");

        // A should depend on B and C, but not D
        assert_eq!(reduced_deps.len(), 2);
        assert!(reduced_deps.contains("B"));
        assert!(reduced_deps.contains("C"));
        assert!(!reduced_deps.contains("D"));
    }

    #[test]
    fn test_get_isolated_nodes_empty_graph() {
        let nodes: Vec<&TestNode> = vec![];
        let graph = DependencyGraph::new(&nodes);

        let isolated = graph.get_isolated_nodes();
        assert_eq!(isolated.len(), 0);
    }

    #[test]
    fn test_get_isolated_nodes_single_node() {
        let node = TestNode::new("alone", vec![]);
        let graph = DependencyGraph::new(&[&node]);

        let isolated = graph.get_isolated_nodes();
        assert_eq!(isolated.len(), 1);
        assert_eq!(isolated[0].id(), "alone");
    }

    #[test]
    fn test_get_isolated_nodes_connected_chain() {
        // A→B→C - no isolated nodes
        let a = TestNode::new("A", vec!["B"]);
        let b = TestNode::new("B", vec!["C"]);
        let c = TestNode::new("C", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c]);
        let isolated = graph.get_isolated_nodes();
        assert_eq!(isolated.len(), 0);
    }

    #[test]
    fn test_get_isolated_nodes_with_isolated() {
        // A→B and C is isolated
        let a = TestNode::new("A", vec!["B"]);
        let b = TestNode::new("B", vec![]);
        let c = TestNode::new("C", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c]);
        let isolated = graph.get_isolated_nodes();
        assert_eq!(isolated.len(), 1);
        assert_eq!(isolated[0].id(), "C");
    }

    #[test]
    fn test_get_isolated_nodes_multiple_isolated() {
        // A→B and C, D are isolated
        let a = TestNode::new("A", vec!["B"]);
        let b = TestNode::new("B", vec![]);
        let c = TestNode::new("C", vec![]);
        let d = TestNode::new("D", vec![]);

        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);
        let isolated = graph.get_isolated_nodes();
        assert_eq!(isolated.len(), 2);
        let isolated_ids: Vec<&str> = isolated.iter().map(|n| n.id()).collect();
        assert!(isolated_ids.contains(&"C"));
        assert!(isolated_ids.contains(&"D"));
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
            ids(&graph.traverse("A", Direction::Dependencies, 1)),
            vec!["B", "C"]
        );
    }

    #[test]
    fn test_traverse_dependencies_respects_depth_bound() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        // Depth 2 reaches C (via B or the shortcut) but stops before D.
        assert_eq!(
            sorted_ids(&graph.traverse("A", Direction::Dependencies, 2)),
            vec!["B", "C"]
        );
        assert_eq!(
            sorted_ids(&graph.traverse("A", Direction::Dependencies, 3)),
            vec!["B", "C", "D"]
        );
    }

    #[test]
    fn test_traverse_dependencies_visits_each_node_once() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        // C is reachable as A→C and A→B→C; it is returned once.
        assert_eq!(
            sorted_ids(&graph.traverse("A", Direction::Dependencies, 0)),
            vec!["B", "C", "D"]
        );
    }

    #[test]
    fn test_traverse_excludes_start_node() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        assert!(graph
            .traverse("A", Direction::Dependencies, 0)
            .iter()
            .all(|node| node.id() != "A"));
        assert!(graph
            .traverse("D", Direction::Dependents, 0)
            .iter()
            .all(|node| node.id() != "D"));
    }

    #[test]
    fn test_traverse_dependents_walks_incoming_edges() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        assert_eq!(
            ids(&graph.traverse("C", Direction::Dependents, 1)),
            vec!["A", "B"]
        );
        assert_eq!(
            sorted_ids(&graph.traverse("D", Direction::Dependents, 0)),
            vec!["A", "B", "C"]
        );
        assert_eq!(
            ids(&graph.traverse("D", Direction::Dependents, 1)),
            vec!["C"]
        );
    }

    #[test]
    fn test_traverse_dependents_matches_transitive_dependents() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        let mut expected: Vec<String> = graph
            .get_transitive_dependents("D")
            .iter()
            .map(|node| node.id().to_string())
            .collect();
        expected.sort();

        assert_eq!(
            sorted_ids(&graph.traverse("D", Direction::Dependents, 0)),
            expected
        );
    }

    #[test]
    fn test_traverse_skips_edges_to_absent_nodes() {
        let a = TestNode::new("A", vec!["missing", "B"]);
        let b = TestNode::new("B", vec![]);
        let graph = DependencyGraph::new(&[&a, &b]);

        assert_eq!(
            ids(&graph.traverse("A", Direction::Dependencies, 0)),
            vec!["B"]
        );
    }

    #[test]
    fn test_traverse_unknown_start_returns_nothing() {
        let a = TestNode::new("A", vec![]);
        let graph = DependencyGraph::new(&[&a]);

        assert!(graph
            .traverse("nonexistent", Direction::Dependencies, 0)
            .is_empty());
    }

    // REQ-4: unlimited forward traversal agrees with the transitive-dependencies
    // primitive on a hand-built diamond, and (below) on arbitrary DAGs.
    #[test]
    fn test_traverse_unlimited_matches_transitive_dependencies() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        let mut expected: Vec<String> = graph
            .get_transitive_dependencies("A")
            .iter()
            .map(|node| node.id().to_string())
            .collect();
        expected.sort();

        assert_eq!(
            sorted_ids(&graph.traverse("A", Direction::Dependencies, 0)),
            expected
        );
    }

    #[test]
    fn test_expand_unfolds_shared_node_once_per_path() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        let forest = graph.expand("A", Direction::Dependencies, 0);
        // A's neighbours, in declaration order.
        assert_eq!(
            forest.iter().map(|e| e.node.id()).collect::<Vec<_>>(),
            vec!["B", "C"]
        );
        assert!(forest.iter().all(|e| e.level == 1));
        // B's subtree re-unfolds C, so C occurs twice overall (and D with it).
        let counts = Expansion::occurrences(&forest);
        assert_eq!(counts.get("C"), Some(&2));
        assert_eq!(counts.get("D"), Some(&2));
        assert_eq!(counts.get("B"), Some(&1));
    }

    #[test]
    fn test_expand_stops_at_depth_bound() {
        let (a, b, c, d) = chain_with_shortcut();
        let graph = DependencyGraph::new(&[&a, &b, &c, &d]);

        let forest = graph.expand("A", Direction::Dependencies, 1);
        assert_eq!(forest.len(), 2);
        assert!(forest.iter().all(|e| e.children.is_empty()));

        let forest = graph.expand("A", Direction::Dependencies, 2);
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
            .expand("A", Direction::Dependencies, 0)
            .iter()
            .map(|expansion| expansion.fold(&render))
            .collect();

        assert_eq!(rendered, vec!["B@1C@2D@3", "C@1D@2"]);
    }

    #[test]
    fn test_get_isolated_nodes_with_issues() {
        // Test with actual Issue type
        let issue1 = Issue::new("Connected 1".to_string(), "Desc".to_string());
        let mut issue2 = Issue::new("Connected 2".to_string(), "Desc".to_string());
        let issue3 = Issue::new("Isolated".to_string(), "Desc".to_string());

        issue2.dependencies.push(issue1.id.clone());

        let issues = vec![&issue1, &issue2, &issue3];
        let graph = DependencyGraph::new(&issues);

        let isolated = graph.get_isolated_nodes();
        assert_eq!(isolated.len(), 1);
        assert_eq!(isolated[0].id, issue3.id);
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
