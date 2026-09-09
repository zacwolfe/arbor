//! Virtual dispatch expansion.
//!
//! This is the part that types alone do not buy you. On the JVM, resolving
//! `PaymentGateway.charge()` to a definition is only half the answer — the
//! call at runtime lands in `StripeGateway.charge()` or `MockGateway.charge()`,
//! and a call graph that stops at the interface reports a blast radius of one
//! for a change that actually reaches every implementation.
//!
//! SCIP records `is_implementation` relationships, so the override hierarchy
//! is available without any analysis of our own. We walk it and synthesise the
//! extra call edges, confidence-weighted by how many candidates there are:
//! one implementation is a certainty, twelve is a shrug.

use crate::edge::SymbolEdge;
use arbor_graph::EdgeKind;
use std::collections::{HashMap, HashSet};

/// Beyond this many implementations, "calls one of these" carries no
/// information and the edges would only inflate centrality. A `Comparable`
/// or `Runnable` in a large codebase has hundreds.
pub const MAX_DISPATCH_FANOUT: usize = 8;

/// Floor for a dispatch edge's confidence. Even at maximum fan-out the edge
/// is a real possibility, so it should not round to "ignore me".
///
/// Deliberately below `1.0 / MAX_DISPATCH_FANOUT` so the `1/n` gradient stays
/// meaningful across the whole permitted range; the floor only exists to stop
/// a future fan-out increase from producing edges that read as noise.
pub const MIN_DISPATCH_CONFIDENCE: f32 = 0.1;

/// How deep to follow the override chain.
///
/// Interface → abstract base → concrete class is three levels and entirely
/// ordinary in Spring code, so stopping at one would miss the implementation
/// that actually runs. Past four the hierarchy is nearly always generic
/// framework plumbing.
const MAX_HIERARCHY_DEPTH: usize = 4;

/// Override hierarchy: for a given symbol, who implements or overrides it.
#[derive(Debug, Default)]
pub struct ImplementationMap {
    supertype_to_impls: HashMap<String, Vec<String>>,
}

impl ImplementationMap {
    /// Builds the map from `(implementing symbol, implemented symbol)` pairs,
    /// i.e. SCIP relationships where `is_implementation` is set.
    pub fn from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut supertype_to_impls: HashMap<String, Vec<String>> = HashMap::new();

        for (implementor, supertype) in pairs {
            if implementor == supertype {
                continue;
            }
            supertype_to_impls
                .entry(supertype)
                .or_default()
                .push(implementor);
        }

        for impls in supertype_to_impls.values_mut() {
            impls.sort();
            impls.dedup();
        }

        Self { supertype_to_impls }
    }

    /// Direct implementors of `symbol`.
    pub fn direct_implementations(&self, symbol: &str) -> &[String] {
        self.supertype_to_impls
            .get(symbol)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Every implementor reachable through the override chain, transitively.
    ///
    /// Excludes `symbol` itself, so an abstract method that is also directly
    /// callable does not appear as its own implementation.
    pub fn transitive_implementations(&self, symbol: &str) -> Vec<String> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut frontier: Vec<&str> = vec![symbol];

        for _ in 0..MAX_HIERARCHY_DEPTH {
            let mut next: Vec<&str> = Vec::new();

            for current in frontier {
                for implementor in self.direct_implementations(current) {
                    if implementor != symbol && seen.insert(implementor.as_str()) {
                        next.push(implementor.as_str());
                    }
                }
            }

            if next.is_empty() {
                break;
            }
            frontier = next;
        }

        let mut result: Vec<String> = seen.into_iter().map(str::to_string).collect();
        result.sort();
        result
    }

    /// How many supertypes have at least one recorded implementor.
    pub fn len(&self) -> usize {
        self.supertype_to_impls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.supertype_to_impls.is_empty()
    }

    /// Synthesises the call edges that virtual dispatch implies.
    ///
    /// Only `Calls` edges are expanded — a type reference to an interface does
    /// not mean the referencing code touches every implementation of it.
    ///
    /// The original edge to the declaring symbol is left in place by the
    /// caller: it is still true, and dropping it would lose the fact that the
    /// code was written against the abstraction.
    pub fn expand(&self, edges: &[SymbolEdge]) -> Vec<SymbolEdge> {
        if self.is_empty() {
            return Vec::new();
        }

        let mut expanded = Vec::new();

        for edge in edges.iter().filter(|e| e.kind == EdgeKind::Calls) {
            let implementations = self.transitive_implementations(&edge.to);

            if implementations.is_empty() || implementations.len() > MAX_DISPATCH_FANOUT {
                continue;
            }

            let confidence = dispatch_confidence(implementations.len());

            for implementation in implementations {
                if implementation == edge.from {
                    continue;
                }

                expanded.push(
                    SymbolEdge::exact(
                        edge.from.clone(),
                        implementation,
                        EdgeKind::Calls,
                        edge.file.clone(),
                        edge.line,
                    )
                    .with_confidence(confidence),
                );
            }
        }

        expanded
    }
}

/// Confidence for a dispatch edge given `n` candidate implementations.
///
/// A single implementation is not a guess at all: the compiler resolved the
/// declaration and there is exactly one body it can reach. Past that, the
/// candidates split the certainty between them.
fn dispatch_confidence(n: usize) -> f32 {
    match n {
        0 => 0.0,
        1 => 1.0,
        n => (1.0 / n as f32).max(MIN_DISPATCH_CONFIDENCE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(from: &str, to: &str) -> SymbolEdge {
        SymbolEdge::exact(from, to, EdgeKind::Calls, "Caller.java", 42)
    }

    #[test]
    fn single_implementation_is_certain() {
        let map = ImplementationMap::from_pairs(vec![(
            "StripeGateway#charge().".to_string(),
            "PaymentGateway#charge().".to_string(),
        )]);

        let expanded = map.expand(&[call("Checkout#pay().", "PaymentGateway#charge().")]);

        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].to, "StripeGateway#charge().");
        assert_eq!(expanded[0].confidence, 1.0);
        // Location is carried over from the call site, not the implementation.
        assert_eq!(expanded[0].file, "Caller.java");
        assert_eq!(expanded[0].line, 42);
    }

    #[test]
    fn multiple_implementations_split_confidence() {
        let map = ImplementationMap::from_pairs(vec![
            (
                "Stripe#charge().".to_string(),
                "Gateway#charge().".to_string(),
            ),
            (
                "Mock#charge().".to_string(),
                "Gateway#charge().".to_string(),
            ),
        ]);

        let expanded = map.expand(&[call("Checkout#pay().", "Gateway#charge().")]);

        assert_eq!(expanded.len(), 2);
        assert!(expanded.iter().all(|e| (e.confidence - 0.5).abs() < 1e-6));
    }

    #[test]
    fn follows_hierarchy_transitively() {
        // Gateway (interface) <- AbstractGateway <- StripeGateway
        let map = ImplementationMap::from_pairs(vec![
            (
                "AbstractGateway#charge().".to_string(),
                "Gateway#charge().".to_string(),
            ),
            (
                "StripeGateway#charge().".to_string(),
                "AbstractGateway#charge().".to_string(),
            ),
        ]);

        let impls = map.transitive_implementations("Gateway#charge().");
        assert_eq!(
            impls,
            vec![
                "AbstractGateway#charge().".to_string(),
                "StripeGateway#charge().".to_string()
            ]
        );
    }

    #[test]
    fn wide_fanout_is_dropped() {
        let pairs: Vec<(String, String)> = (0..MAX_DISPATCH_FANOUT + 1)
            .map(|i| (format!("Impl{i}#run()."), "Runnable#run().".to_string()))
            .collect();
        let map = ImplementationMap::from_pairs(pairs);

        let expanded = map.expand(&[call("Scheduler#tick().", "Runnable#run().")]);
        assert!(expanded.is_empty());
    }

    #[test]
    fn confidence_never_rounds_to_nothing() {
        assert_eq!(dispatch_confidence(MAX_DISPATCH_FANOUT), 0.125);
        assert!(dispatch_confidence(1000) >= MIN_DISPATCH_CONFIDENCE);
        assert_eq!(dispatch_confidence(0), 0.0);
    }

    #[test]
    fn non_call_edges_are_not_expanded() {
        let map =
            ImplementationMap::from_pairs(vec![("Stripe#".to_string(), "Gateway#".to_string())]);

        let type_ref = SymbolEdge::exact(
            "Checkout#",
            "Gateway#",
            EdgeKind::UsesType,
            "Checkout.java",
            3,
        );

        assert!(map.expand(&[type_ref]).is_empty());
    }

    #[test]
    fn cyclic_hierarchy_terminates() {
        let map = ImplementationMap::from_pairs(vec![
            ("A#run().".to_string(), "B#run().".to_string()),
            ("B#run().".to_string(), "A#run().".to_string()),
        ]);

        let impls = map.transitive_implementations("A#run().");
        assert_eq!(impls, vec!["B#run().".to_string()]);
    }

    #[test]
    fn self_referencing_pairs_are_ignored() {
        let map = ImplementationMap::from_pairs(vec![("A#".to_string(), "A#".to_string())]);
        assert!(map.is_empty());
    }
}
