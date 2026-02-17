//! DAG-based step scheduler for multi-step orchestrations.
//!
//! The [`DagScheduler`] validates that the dependency graph is acyclic (via
//! Kahn's algorithm) and then drives step execution: it computes in-degrees,
//! yields "ready" steps whose dependencies are met, and propagates failure
//! downstream.

use std::collections::{HashMap, HashSet, VecDeque};

use super::manager::ManagerError;

/// State of a step in the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepState {
    /// Waiting for dependencies.
    Pending,
    /// All dependencies met, ready to execute.
    Ready,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Completed,
    /// Failed.
    Failed,
    /// Blocked by a failed dependency.
    Blocked,
}

/// DAG scheduler for orchestration steps.
///
/// Manages step states and dependency tracking, determines which steps are
/// ready to run, and cascades failures to downstream dependents.
#[derive(Debug)]
pub struct DagScheduler {
    /// All step IDs in the orchestration.
    steps: Vec<String>,
    /// Reverse dependencies: `step_id` -> list of step IDs that depend on it.
    dependents: HashMap<String, Vec<String>>,
    /// Current state of each step.
    states: HashMap<String, StepState>,
    /// Remaining in-degree for each step (number of unsatisfied dependencies).
    in_degrees: HashMap<String, usize>,
}

impl DagScheduler {
    /// Create a new scheduler and validate the DAG.
    ///
    /// Returns an error if the dependency graph contains a cycle.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        steps: Vec<String>,
        deps: HashMap<String, Vec<String>>,
    ) -> Result<Self, ManagerError> {
        // Build reverse dependency map
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        for step in &steps {
            dependents.entry(step.clone()).or_default();
        }
        for (step, dep_list) in &deps {
            for dep in dep_list {
                dependents
                    .entry(dep.clone())
                    .or_default()
                    .push(step.clone());
            }
        }

        // Compute in-degrees
        let mut in_degrees: HashMap<String, usize> = HashMap::new();
        for step in &steps {
            let count = deps.get(step).map_or(0, Vec::len);
            in_degrees.insert(step.clone(), count);
        }

        // Validate acyclicity via Kahn's algorithm
        validate_dag(&steps, &deps)?;

        // Initialize states
        let mut states: HashMap<String, StepState> = HashMap::new();
        for step in &steps {
            let deg = in_degrees.get(step).copied().unwrap_or(0);
            if deg == 0 {
                states.insert(step.clone(), StepState::Ready);
            } else {
                states.insert(step.clone(), StepState::Pending);
            }
        }

        Ok(Self {
            steps,
            dependents,
            states,
            in_degrees,
        })
    }

    /// Get the next batch of steps that are ready to execute.
    ///
    /// Returns step IDs whose state is `Ready`. These can be started
    /// concurrently.
    pub fn next_ready(&self) -> Vec<String> {
        self.steps
            .iter()
            .filter(|s| self.states.get(*s) == Some(&StepState::Ready))
            .cloned()
            .collect()
    }

    /// Mark a step as currently running.
    pub fn mark_running(&mut self, step_id: &str) {
        if let Some(state) = self.states.get_mut(step_id) {
            *state = StepState::Running;
        }
    }

    /// Mark a step as completed.
    ///
    /// Decrements in-degrees of downstream steps and moves newly-unblocked
    /// steps to `Ready`. Returns the IDs of newly-ready downstream steps.
    pub fn mark_completed(&mut self, step_id: &str) -> Vec<String> {
        if let Some(state) = self.states.get_mut(step_id) {
            *state = StepState::Completed;
        }

        let mut newly_ready = Vec::new();

        // Decrement in-degrees of dependents
        if let Some(downstream) = self.dependents.get(step_id).cloned() {
            for ds in &downstream {
                if let Some(deg) = self.in_degrees.get_mut(ds) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0
                        && let Some(state) = self.states.get_mut(ds)
                        && *state == StepState::Pending
                    {
                        *state = StepState::Ready;
                        newly_ready.push(ds.clone());
                    }
                }
            }
        }

        newly_ready
    }

    /// Mark a step as failed.
    ///
    /// Cascades failure to all downstream dependents (transitively), marking
    /// them as `Blocked`. Returns the IDs of all blocked steps.
    pub fn mark_failed(&mut self, step_id: &str) -> Vec<String> {
        if let Some(state) = self.states.get_mut(step_id) {
            *state = StepState::Failed;
        }

        // Cascade: BFS through dependents
        let mut blocked = Vec::new();
        let mut queue: VecDeque<String> = VecDeque::new();

        if let Some(downstream) = self.dependents.get(step_id) {
            for ds in downstream {
                queue.push_back(ds.clone());
            }
        }

        let mut visited: HashSet<String> = HashSet::new();
        while let Some(ds) = queue.pop_front() {
            if !visited.insert(ds.clone()) {
                continue;
            }

            if let Some(state) = self.states.get_mut(&ds)
                && (*state == StepState::Pending || *state == StepState::Ready)
            {
                *state = StepState::Blocked;
                blocked.push(ds.clone());
            }

            // Continue cascading
            if let Some(further_downstream) = self.dependents.get(&ds) {
                for fds in further_downstream {
                    queue.push_back(fds.clone());
                }
            }
        }

        blocked
    }

    /// Check if all steps have reached a terminal state.
    pub fn is_complete(&self) -> bool {
        self.states.values().all(|s| {
            matches!(
                s,
                StepState::Completed | StepState::Failed | StepState::Blocked
            )
        })
    }

    /// Get the state of a specific step.
    pub fn step_state(&self, step_id: &str) -> Option<StepState> {
        self.states.get(step_id).copied()
    }

    /// Get all step IDs currently in `Running` state.
    pub fn running_ids(&self) -> Vec<String> {
        self.steps
            .iter()
            .filter(|s| self.states.get(*s) == Some(&StepState::Running))
            .cloned()
            .collect()
    }

    /// Get the total number of steps.
    pub const fn total_steps(&self) -> usize {
        self.steps.len()
    }

    /// Count steps in each state.
    pub fn counts(&self) -> HashMap<StepState, usize> {
        let mut counts: HashMap<StepState, usize> = HashMap::new();
        for state in self.states.values() {
            *counts.entry(*state).or_insert(0) += 1;
        }
        counts
    }
}

/// Validate that the dependency graph is a DAG (no cycles) using Kahn's algorithm.
fn validate_dag(steps: &[String], deps: &HashMap<String, Vec<String>>) -> Result<(), ManagerError> {
    let step_set: HashSet<&str> = steps.iter().map(String::as_str).collect();

    // Validate that all dependencies reference known steps
    for (step, dep_list) in deps {
        for dep in dep_list {
            if !step_set.contains(dep.as_str()) {
                return Err(ManagerError::Validation {
                    message: format!("Step '{step}' depends on unknown step '{dep}'"),
                });
            }
        }
        // Self-dependency check
        if dep_list.contains(step) {
            return Err(ManagerError::Validation {
                message: format!("Step '{step}' depends on itself"),
            });
        }
    }

    // Kahn's algorithm: compute in-degrees, process zero-degree nodes
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

    for step in steps {
        in_degree.entry(step.as_str()).or_insert(0);
        adj.entry(step.as_str()).or_default();
    }

    for (step, dep_list) in deps {
        for dep in dep_list {
            adj.entry(dep.as_str()).or_default().push(step.as_str());
            *in_degree.entry(step.as_str()).or_insert(0) += 1;
        }
    }

    let mut queue: VecDeque<&str> = VecDeque::new();
    for (step, &deg) in &in_degree {
        if deg == 0 {
            queue.push_back(step);
        }
    }

    let mut processed = 0usize;
    while let Some(step) = queue.pop_front() {
        processed += 1;
        if let Some(neighbors) = adj.get(step) {
            for &neighbor in neighbors {
                if let Some(deg) = in_degree.get_mut(neighbor) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
    }

    if processed != steps.len() {
        return Err(ManagerError::Validation {
            message: "Dependency graph contains a cycle".to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn steps(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    fn deps(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(k, v)| {
                (
                    (*k).to_string(),
                    v.iter().map(|s| (*s).to_string()).collect(),
                )
            })
            .collect()
    }

    // =========================================================================
    // DAG Validation
    // =========================================================================

    #[test]
    fn rejects_cycle() {
        let s = steps(&["a", "b", "c"]);
        let d = deps(&[("a", &["c"]), ("b", &["a"]), ("c", &["b"])]);
        let result = DagScheduler::new(s, d);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cycle"), "Error should mention cycle: {err}");
    }

    #[test]
    fn rejects_self_dependency() {
        let s = steps(&["a"]);
        let d = deps(&[("a", &["a"])]);
        let result = DagScheduler::new(s, d);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("depends on itself"),
            "Error should mention self-dependency: {err}"
        );
    }

    #[test]
    fn rejects_unknown_dependency() {
        let s = steps(&["a", "b"]);
        let d = deps(&[("a", &["nonexistent"])]);
        let result = DagScheduler::new(s, d);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown step"),
            "Error should mention unknown step: {err}"
        );
    }

    #[test]
    fn accepts_valid_dag() {
        let s = steps(&["a", "b", "c"]);
        let d = deps(&[("b", &["a"]), ("c", &["b"])]);
        let result = DagScheduler::new(s, d);
        assert!(result.is_ok());
    }

    #[test]
    fn accepts_empty_graph() {
        let s: Vec<String> = vec![];
        let d: HashMap<String, Vec<String>> = HashMap::new();
        let sched = DagScheduler::new(s, d).unwrap();
        assert!(sched.is_complete());
    }

    #[test]
    fn accepts_no_dependencies() {
        let s = steps(&["a", "b", "c"]);
        let d: HashMap<String, Vec<String>> = HashMap::new();
        let sched = DagScheduler::new(s, d).unwrap();
        let ready = sched.next_ready();
        // All should be ready
        assert_eq!(ready.len(), 3);
    }

    // =========================================================================
    // PARALLEL Strategy
    // =========================================================================

    #[test]
    fn parallel_all_steps_ready_initially() {
        let s = steps(&["a", "b", "c"]);
        let d: HashMap<String, Vec<String>> = HashMap::new();
        let sched = DagScheduler::new(s, d).unwrap();
        let mut ready = sched.next_ready();
        ready.sort();
        assert_eq!(ready, vec!["a", "b", "c"]);
    }

    #[test]
    fn parallel_complete_all() {
        let s = steps(&["a", "b", "c"]);
        let d: HashMap<String, Vec<String>> = HashMap::new();
        let mut sched = DagScheduler::new(s, d).unwrap();

        for id in &["a", "b", "c"] {
            sched.mark_running(id);
        }
        for id in &["a", "b", "c"] {
            sched.mark_completed(id);
        }

        assert!(sched.is_complete());
        let counts = sched.counts();
        assert_eq!(counts.get(&StepState::Completed), Some(&3));
    }

    // =========================================================================
    // SEQUENTIAL Strategy
    // =========================================================================

    #[test]
    fn sequential_one_at_a_time() {
        // a -> b -> c
        let s = steps(&["a", "b", "c"]);
        let d = deps(&[("b", &["a"]), ("c", &["b"])]);
        let mut sched = DagScheduler::new(s, d).unwrap();

        // Only 'a' should be ready
        assert_eq!(sched.next_ready(), vec!["a"]);

        sched.mark_running("a");
        assert!(sched.next_ready().is_empty());

        let newly_ready = sched.mark_completed("a");
        assert_eq!(newly_ready, vec!["b"]);
        assert_eq!(sched.next_ready(), vec!["b"]);

        sched.mark_running("b");
        let newly_ready = sched.mark_completed("b");
        assert_eq!(newly_ready, vec!["c"]);
        assert_eq!(sched.next_ready(), vec!["c"]);

        sched.mark_running("c");
        sched.mark_completed("c");
        assert!(sched.is_complete());
    }

    // =========================================================================
    // DAG Strategy
    // =========================================================================

    #[test]
    fn dag_diamond_pattern() {
        let s = steps(&["a", "b", "c", "d"]);
        let d = deps(&[("b", &["a"]), ("c", &["a"]), ("d", &["b", "c"])]);
        let mut sched = DagScheduler::new(s, d).unwrap();

        assert_eq!(sched.next_ready(), vec!["a"]);

        sched.mark_running("a");
        let newly_ready = sched.mark_completed("a");
        let mut ready = newly_ready;
        ready.sort();
        assert_eq!(ready, vec!["b", "c"]);

        sched.mark_running("b");
        sched.mark_running("c");

        sched.mark_completed("b");
        assert!(sched.next_ready().is_empty());

        let newly_ready = sched.mark_completed("c");
        assert_eq!(newly_ready, vec!["d"]);

        sched.mark_running("d");
        sched.mark_completed("d");
        assert!(sched.is_complete());
    }

    // =========================================================================
    // Failure Cascading
    // =========================================================================

    #[test]
    fn failure_cascades_to_dependents() {
        let s = steps(&["a", "b", "c"]);
        let d = deps(&[("b", &["a"]), ("c", &["b"])]);
        let mut sched = DagScheduler::new(s, d).unwrap();

        sched.mark_running("a");
        let blocked = sched.mark_failed("a");
        assert_eq!(blocked.len(), 2);
        assert!(blocked.contains(&"b".to_string()));
        assert!(blocked.contains(&"c".to_string()));

        assert_eq!(sched.step_state("a"), Some(StepState::Failed));
        assert_eq!(sched.step_state("b"), Some(StepState::Blocked));
        assert_eq!(sched.step_state("c"), Some(StepState::Blocked));
        assert!(sched.is_complete());
    }

    #[test]
    fn failure_does_not_block_independent_steps() {
        let s = steps(&["a", "b", "c", "d"]);
        let d = deps(&[("c", &["a", "b"])]);
        let mut sched = DagScheduler::new(s, d).unwrap();

        let mut ready = sched.next_ready();
        ready.sort();
        assert_eq!(ready, vec!["a", "b", "d"]);

        sched.mark_running("a");
        sched.mark_running("b");
        sched.mark_running("d");

        let blocked = sched.mark_failed("a");
        assert_eq!(blocked, vec!["c"]);

        sched.mark_completed("d");
        sched.mark_completed("b");

        assert!(sched.is_complete());
        assert_eq!(sched.step_state("d"), Some(StepState::Completed));
        assert_eq!(sched.step_state("c"), Some(StepState::Blocked));
    }

    #[test]
    fn failure_cascades_transitively() {
        let s = steps(&["a", "b", "c", "d"]);
        let d = deps(&[("b", &["a"]), ("c", &["b"]), ("d", &["c"])]);
        let mut sched = DagScheduler::new(s, d).unwrap();

        sched.mark_running("a");
        let blocked = sched.mark_failed("a");
        assert_eq!(blocked.len(), 3);
        assert!(blocked.contains(&"b".to_string()));
        assert!(blocked.contains(&"c".to_string()));
        assert!(blocked.contains(&"d".to_string()));
    }

    // =========================================================================
    // State queries
    // =========================================================================

    #[test]
    fn step_state_returns_none_for_unknown() {
        let s = steps(&["a"]);
        let d: HashMap<String, Vec<String>> = HashMap::new();
        let sched = DagScheduler::new(s, d).unwrap();
        assert!(sched.step_state("nonexistent").is_none());
    }

    #[test]
    fn running_ids_tracks_correctly() {
        let s = steps(&["a", "b"]);
        let d: HashMap<String, Vec<String>> = HashMap::new();
        let mut sched = DagScheduler::new(s, d).unwrap();

        assert!(sched.running_ids().is_empty());

        sched.mark_running("a");
        assert_eq!(sched.running_ids(), vec!["a"]);

        sched.mark_running("b");
        let mut ids = sched.running_ids();
        ids.sort();
        assert_eq!(ids, vec!["a", "b"]);

        sched.mark_completed("a");
        assert_eq!(sched.running_ids(), vec!["b"]);
    }

    #[test]
    fn total_steps_correct() {
        let s = steps(&["a", "b", "c"]);
        let d: HashMap<String, Vec<String>> = HashMap::new();
        let sched = DagScheduler::new(s, d).unwrap();
        assert_eq!(sched.total_steps(), 3);
    }

    #[test]
    fn counts_reflect_state() {
        let s = steps(&["a", "b", "c"]);
        let d = deps(&[("c", &["a", "b"])]);
        let mut sched = DagScheduler::new(s, d).unwrap();

        let counts = sched.counts();
        assert_eq!(counts.get(&StepState::Ready), Some(&2));
        assert_eq!(counts.get(&StepState::Pending), Some(&1));

        sched.mark_running("a");
        let counts = sched.counts();
        assert_eq!(counts.get(&StepState::Running), Some(&1));
        assert_eq!(counts.get(&StepState::Ready), Some(&1));
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn single_step_no_deps() {
        let s = steps(&["only"]);
        let d: HashMap<String, Vec<String>> = HashMap::new();
        let mut sched = DagScheduler::new(s, d).unwrap();

        assert_eq!(sched.next_ready(), vec!["only"]);
        sched.mark_running("only");
        sched.mark_completed("only");
        assert!(sched.is_complete());
    }

    #[test]
    fn multiple_roots_converge() {
        let s = steps(&["a", "b", "c", "d"]);
        let d = deps(&[("d", &["a", "b", "c"])]);
        let mut sched = DagScheduler::new(s, d).unwrap();

        let mut ready = sched.next_ready();
        ready.sort();
        assert_eq!(ready, vec!["a", "b", "c"]);

        sched.mark_running("a");
        sched.mark_running("b");
        sched.mark_running("c");

        sched.mark_completed("a");
        assert!(sched.next_ready().is_empty());

        sched.mark_completed("b");
        assert!(sched.next_ready().is_empty());

        let newly_ready = sched.mark_completed("c");
        assert_eq!(newly_ready, vec!["d"]);
    }

    #[test]
    fn two_node_cycle_detected() {
        let s = steps(&["a", "b"]);
        let d = deps(&[("a", &["b"]), ("b", &["a"])]);
        let result = DagScheduler::new(s, d);
        assert!(result.is_err());
    }

    #[test]
    fn complex_dag_validates() {
        let s = steps(&["a", "b", "c", "d", "e", "f"]);
        let d = deps(&[
            ("b", &["a"]),
            ("c", &["a"]),
            ("d", &["b"]),
            ("e", &["c"]),
            ("f", &["d", "e"]),
        ]);
        let sched = DagScheduler::new(s, d).unwrap();
        assert_eq!(sched.total_steps(), 6);
        assert_eq!(sched.next_ready(), vec!["a"]);
    }
}
