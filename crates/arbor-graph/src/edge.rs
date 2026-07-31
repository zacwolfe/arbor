//! Edge types for the code graph.
//!
//! Edges represent relationships between code entities. We keep
//! the set of edge kinds focused on what's useful for understanding
//! code architecture.

use serde::{Deserialize, Serialize};

/// The type of relationship between two code entities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Function A calls function B.
    Calls,

    /// Module A imports from module B.
    Imports,

    /// Class A extends class B.
    Extends,

    /// Class implements interface/trait.
    Implements,

    /// Type references another type.
    UsesType,

    /// General reference to a symbol.
    References,

    /// Container relationship (class contains method).
    Contains,

    /// Control flow: statement A flows to statement B.
    /// Used for CFG (Control Flow Graph) edges.
    FlowsTo,

    /// Data dependency: variable use depends on definition.
    /// Used for DFA (Data Flow Analysis) edges.
    DataDependency,
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Calls => "calls",
            Self::Imports => "imports",
            Self::Extends => "extends",
            Self::Implements => "implements",
            Self::UsesType => "uses_type",
            Self::References => "references",
            Self::Contains => "contains",
            Self::FlowsTo => "flows_to",
            Self::DataDependency => "data_dependency",
        };
        write!(f, "{}", s)
    }
}

/// Confidence assumed for edges that predate confidence tracking.
///
/// Graphs cached by an older build deserialize without the field; treating
/// them as certain preserves their previous behaviour exactly.
fn default_confidence() -> f32 {
    1.0
}

/// An edge in the code graph with location info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// The kind of relationship.
    pub kind: EdgeKind,

    /// File where this edge originates.
    pub file: Option<String>,

    /// Line number where this edge originates.
    pub line: Option<u32>,

    /// How sure we are this edge is real, in `[0.0, 1.0]`.
    ///
    /// An exact fully-qualified match is `1.0`. A bare name matched only by
    /// same-directory locality is far weaker, and an ambiguous match weaker
    /// still. Consumers that need certainty should filter on this rather than
    /// treating every edge as proven — see [`Edge::is_confident`].
    #[serde(default = "default_confidence")]
    pub confidence: f32,
}

impl Edge {
    /// Edges at or above this confidence are treated as established fact.
    pub const CONFIDENT_THRESHOLD: f32 = 0.75;

    /// Creates a new edge with full confidence.
    pub fn new(kind: EdgeKind) -> Self {
        Self {
            kind,
            file: None,
            line: None,
            confidence: 1.0,
        }
    }

    /// Creates an edge with location info.
    pub fn with_location(kind: EdgeKind, file: impl Into<String>, line: u32) -> Self {
        Self {
            kind,
            file: Some(file.into()),
            line: Some(line),
            confidence: 1.0,
        }
    }

    /// Sets the confidence, clamped to `[0.0, 1.0]`.
    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Whether this edge is strong enough to treat as certain.
    pub fn is_confident(&self) -> bool {
        self.confidence >= Self::CONFIDENT_THRESHOLD
    }
}

/// A simplified edge for graph export/visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
}
