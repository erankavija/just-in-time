//! Dependency graph operations and validation.
//!
//! Provides DAG enforcement, cycle detection, and graph traversal operations.
//!
//! The graph module provides a generic `DependencyGraph<T>` that works with any
//! type implementing the `GraphNode` trait. This allows the same DAG algorithms
//! to be used for issues, tasks, packages, or any other dependency relationships.

use std::collections::{HashMap, HashSet};
use thiserror::Error;

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
}
