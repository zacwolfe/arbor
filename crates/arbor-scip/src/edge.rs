//! An edge still expressed in SCIP symbol strings.
//!
//! Ingest works in symbol space and only converts to graph node IDs at the
//! very end, because dispatch expansion needs to look symbols up by their
//! supertype relationships — something node IDs cannot express.

use arbor_graph::EdgeKind;

/// A relationship between two SCIP symbols.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolEdge {
    /// SCIP symbol of the enclosing definition the reference occurs in.
    pub from: String,

    /// SCIP symbol being referenced.
    pub to: String,

    pub kind: EdgeKind,

    /// Document-relative path of the referencing site.
    pub file: String,

    /// 1-indexed line of the referencing site.
    pub line: u32,

    /// `1.0` for anything the compiler resolved outright. Only dispatch
    /// expansion produces anything lower — see [`crate::dispatch`].
    pub confidence: f32,
}

impl SymbolEdge {
    /// A compiler-resolved edge: exactly what the index said, no inference.
    pub fn exact(
        from: impl Into<String>,
        to: impl Into<String>,
        kind: EdgeKind,
        file: impl Into<String>,
        line: u32,
    ) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind,
            file: file.into(),
            line,
            confidence: 1.0,
        }
    }

    /// Same edge, reduced certainty.
    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }
}
