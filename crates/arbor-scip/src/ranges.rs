//! Range decoding and "which definition is this reference inside?".
//!
//! SCIP records every reference with a position but not always with an owner.
//! An edge needs an owner: `UserService.validate` calls `Repo.findOne`, not
//! "UserService.java calls Repo.findOne". This module recovers that owner.
//!
//! # Two range encodings, both live
//!
//! SCIP carries positions two ways. The original `repeated int32 range` traded
//! type safety for payload size and the schema marks it deprecated in favour of
//! the typed `single_line_range` / `multi_line_range` oneof. **"Deprecated" here
//! describes the schema, not the field's use in practice, and neither encoding
//! is safe to drop.** Measured occurrence counts per producer:
//!
//! | producer | typed | array |
//! |---|---|---|
//! | `scip-java` | all | none |
//! | `rust-analyzer` | none | 69,618 |
//! | `scip-typescript` | none | 1,385 |
//! | `scip-python` | none | 1,209 |
//!
//! So `scip-java` is the lone typed-only producer, and the "deprecated" array is
//! the *only* encoding every non-JVM indexer measured emits. Reading just one
//! encoding does not degrade — it yields a graph with **zero nodes**, because
//! every definition is skipped for having no readable position. [`decode_span`]
//! is therefore load-bearing for Rust, TypeScript and Python, and pruning it as
//! dead code breaks those three outright.
//!
//! [`occurrence_span`] tries typed first, as the schema requires, and falls back
//! to the array.

use scip::types::{occurrence, MultiLineRange, Occurrence, SingleLineRange};

/// An inclusive, 1-indexed line span, matching [`arbor_core::CodeNode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start_line: u32,
    pub end_line: u32,
}

impl Span {
    /// Number of lines covered. Used to prefer the innermost of several
    /// nested enclosing definitions.
    fn height(&self) -> u32 {
        self.end_line.saturating_sub(self.start_line)
    }

    fn contains(&self, line: u32) -> bool {
        line >= self.start_line && line <= self.end_line
    }
}

/// Decodes the schema-deprecated SCIP range array into a 1-indexed [`Span`].
///
/// The array packs a range as either `[startLine, startChar, endChar]` when it
/// sits on one line, or `[startLine, startChar, endLine, endChar]` when it does
/// not. Lines are 0-indexed there and 1-indexed here.
///
/// **Not a compatibility shim for old indexes.** `rust-analyzer`,
/// `scip-typescript` and `scip-python` emit this encoding and nothing else
/// today, so this function is the only reason a Rust, TypeScript or Python index
/// produces any nodes at all — see the module docs for the counts. Prefer
/// [`occurrence_span`], which handles both encodings in the order the schema
/// requires.
pub fn decode_span(range: &[i32]) -> Option<Span> {
    let (start, end) = match range {
        [start_line, _start_char, _end_char] => (*start_line, *start_line),
        [start_line, _start_char, end_line, _end_char] => (*start_line, *end_line),
        _ => return None,
    };

    span_from_lines(start, end)
}

/// Builds a 1-indexed [`Span`] from 0-indexed start/end lines.
fn span_from_lines(start: i32, end: i32) -> Option<Span> {
    if start < 0 || end < start {
        return None;
    }

    Some(Span {
        start_line: start as u32 + 1,
        end_line: end as u32 + 1,
    })
}

fn span_from_single_line(range: &SingleLineRange) -> Option<Span> {
    span_from_lines(range.line, range.line)
}

fn span_from_multi_line(range: &MultiLineRange) -> Option<Span> {
    span_from_lines(range.start_line, range.end_line)
}

/// The span of an occurrence itself.
///
/// Prefers the typed encoding, as the schema requires: `typed_range` takes
/// precedence over the `range` array. The fallback is not a legacy path — it is
/// what every non-JVM indexer measured actually uses, so both arms carry real
/// traffic.
pub fn occurrence_span(occurrence: &Occurrence) -> Option<Span> {
    match &occurrence.typed_range {
        Some(occurrence::Typed_range::SingleLineRange(range)) => span_from_single_line(range),
        Some(occurrence::Typed_range::MultiLineRange(range)) => span_from_multi_line(range),
        // Also the path every non-JVM indexer takes, not just an unknown-oneof
        // guard: the array is what rust-analyzer, scip-typescript and
        // scip-python emit. Dropping the node here would empty their graphs.
        _ => decode_span(&occurrence.range),
    }
}

/// The span of an occurrence's *body*, where the indexer supplied one.
///
/// This is what makes enclosing-symbol attribution exact rather than
/// positional, so it is worth reading from whichever encoding is present.
pub fn occurrence_enclosing_span(occurrence: &Occurrence) -> Option<Span> {
    match &occurrence.typed_enclosing_range {
        Some(occurrence::Typed_enclosing_range::SingleLineEnclosingRange(range)) => {
            span_from_single_line(range)
        }
        Some(occurrence::Typed_enclosing_range::MultiLineEnclosingRange(range)) => {
            span_from_multi_line(range)
        }
        _ => decode_span(&occurrence.enclosing_range),
    }
}

/// Answers "which definition encloses line N?" for a single document.
pub struct EnclosingIndex {
    /// Definitions whose full body extent the indexer told us. Authoritative.
    bodies: Vec<(Span, String)>,

    /// Every definition's start line, ascending. The fallback for indexers
    /// that emit no `enclosing_range`, where the best available answer is
    /// "the last definition to open before this line".
    starts: Vec<(u32, String)>,
}

impl EnclosingIndex {
    /// Builds the index from `(definition span, body span, symbol)` triples.
    ///
    /// `body` is the indexer's `enclosing_range` where it supplied one.
    pub fn build(definitions: impl IntoIterator<Item = (Span, Option<Span>, String)>) -> Self {
        let mut bodies = Vec::new();
        let mut starts = Vec::new();

        for (name_span, body_span, symbol) in definitions {
            if let Some(body) = body_span {
                bodies.push((body, symbol.clone()));
            }
            starts.push((name_span.start_line, symbol));
        }

        // Innermost-first: when spans nest, the shortest one is the real owner
        // of the line. Sorting once here keeps `resolve` a linear scan over an
        // already-ordered list.
        bodies.sort_by_key(|(span, _)| span.height());
        starts.sort_by_key(|(line, _)| *line);

        Self { bodies, starts }
    }

    /// The symbol whose definition encloses `line`, if any.
    pub fn resolve(&self, line: u32) -> Option<&str> {
        if self.has_body_extents() {
            // The indexer told us the actual extents, so a line outside all of
            // them genuinely has no owner. Falling back to a guess here would
            // attribute file-level declarations to whichever definition
            // happened to appear above them.
            return self
                .bodies
                .iter()
                .find(|(span, _)| span.contains(line))
                .map(|(_, symbol)| symbol.as_str());
        }

        // No body extents at all. Walk back to the nearest definition that
        // starts at or before this line. This is a heuristic and it is wrong
        // for a reference sitting between two definitions (a field initialiser
        // after the last method, say) — but it is right for the common case of
        // a statement inside a method body, and the alternative is dropping
        // every edge in the document.
        //
        // If only *some* definitions carry extents, the authoritative branch
        // above wins and references inside the rest go unattributed. That is
        // reported as `references_unattributed` rather than papered over.
        self.starts
            .iter()
            .rev()
            .find(|(start, _)| *start <= line)
            .map(|(_, symbol)| symbol.as_str())
    }

    /// Whether the authoritative body extents were available.
    ///
    /// Reported by the ingest stats so a user can tell an exact result from a
    /// heuristic one instead of having to trust the graph blindly.
    pub fn has_body_extents(&self) -> bool {
        !self.bodies.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_single_line_range() {
        let span = decode_span(&[9, 4, 20]).unwrap();
        assert_eq!(span.start_line, 10);
        assert_eq!(span.end_line, 10);
    }

    #[test]
    fn decodes_multi_line_range() {
        let span = decode_span(&[9, 4, 19, 5]).unwrap();
        assert_eq!(span.start_line, 10);
        assert_eq!(span.end_line, 20);
    }

    #[test]
    fn rejects_malformed_range() {
        assert!(decode_span(&[]).is_none());
        assert!(decode_span(&[1, 2]).is_none());
        assert!(decode_span(&[-1, 0, 5]).is_none());
        // end before start is not a range
        assert!(decode_span(&[10, 0, 5, 0]).is_none());
    }

    fn span(start: u32, end: u32) -> Span {
        Span {
            start_line: start,
            end_line: end,
        }
    }

    #[test]
    fn body_extents_win_and_prefer_innermost() {
        let index = EnclosingIndex::build(vec![
            (span(1, 1), Some(span(1, 100)), "Class".to_string()),
            (span(10, 10), Some(span(10, 20)), "Class.method".to_string()),
        ]);

        assert!(index.has_body_extents());
        assert_eq!(index.resolve(15), Some("Class.method"));
        // Outside the method but inside the class.
        assert_eq!(index.resolve(50), Some("Class"));
        assert_eq!(index.resolve(200), None);
    }

    #[test]
    fn falls_back_to_nearest_preceding_definition() {
        let index = EnclosingIndex::build(vec![
            (span(10, 10), None, "first".to_string()),
            (span(30, 30), None, "second".to_string()),
        ]);

        assert!(!index.has_body_extents());
        assert_eq!(index.resolve(15), Some("first"));
        assert_eq!(index.resolve(35), Some("second"));
        // Before any definition there is nothing to attribute to.
        assert_eq!(index.resolve(5), None);
    }

    fn single_line(line: i32) -> SingleLineRange {
        let mut range = SingleLineRange::new();
        range.line = line;
        range.start_character = 4;
        range.end_character = 20;
        range
    }

    fn multi_line(start: i32, end: i32) -> MultiLineRange {
        let mut range = MultiLineRange::new();
        range.start_line = start;
        range.start_character = 4;
        range.end_line = end;
        range.end_character = 5;
        range
    }

    /// `scip-java` emits ONLY the typed encoding. Reading just the array form
    /// yields a graph with zero nodes on every JVM project, so this is the case
    /// that must not regress — and its mirror image below covers the three
    /// indexers that emit only the array.
    #[test]
    fn reads_typed_single_line_range() {
        let mut occurrence = Occurrence::new();
        occurrence.typed_range = Some(occurrence::Typed_range::SingleLineRange(single_line(9)));

        let span = occurrence_span(&occurrence).expect("typed range must be read");
        assert_eq!(span.start_line, 10);
        assert_eq!(span.end_line, 10);
    }

    #[test]
    fn reads_typed_multi_line_range() {
        let mut occurrence = Occurrence::new();
        occurrence.typed_range = Some(occurrence::Typed_range::MultiLineRange(multi_line(9, 19)));

        let span = occurrence_span(&occurrence).expect("typed range must be read");
        assert_eq!(span.start_line, 10);
        assert_eq!(span.end_line, 20);
    }

    #[test]
    fn reads_typed_enclosing_ranges() {
        let mut single = Occurrence::new();
        single.typed_enclosing_range = Some(
            occurrence::Typed_enclosing_range::SingleLineEnclosingRange(single_line(2)),
        );
        assert_eq!(
            occurrence_enclosing_span(&single).unwrap(),
            Span {
                start_line: 3,
                end_line: 3
            }
        );

        let mut multi = Occurrence::new();
        multi.typed_enclosing_range = Some(
            occurrence::Typed_enclosing_range::MultiLineEnclosingRange(multi_line(13, 19)),
        );
        assert_eq!(
            occurrence_enclosing_span(&multi).unwrap(),
            Span {
                start_line: 14,
                end_line: 20
            }
        );
    }

    /// The array encoding is what `rust-analyzer` (69,618 occurrences),
    /// `scip-typescript` (1,385) and `scip-python` (1,209) emit — and they emit
    /// nothing else. This is not a compatibility test for old indexes; it is the
    /// only path those three languages take, and without it their graphs are
    /// empty.
    #[test]
    fn reads_the_array_encoding_every_non_jvm_indexer_emits() {
        let mut occurrence = Occurrence::new();
        occurrence.range = vec![9, 4, 19, 5];
        occurrence.enclosing_range = vec![9, 4, 29, 1];

        assert_eq!(occurrence_span(&occurrence).unwrap().end_line, 20);
        assert_eq!(occurrence_enclosing_span(&occurrence).unwrap().end_line, 30);
    }

    #[test]
    fn typed_range_wins_when_a_producer_writes_both() {
        // The schema says the typed encoding takes precedence, so a producer
        // that writes both must not have the array value preferred.
        let mut occurrence = Occurrence::new();
        occurrence.range = vec![99, 0, 10];
        occurrence.typed_range = Some(occurrence::Typed_range::SingleLineRange(single_line(9)));

        assert_eq!(occurrence_span(&occurrence).unwrap().start_line, 10);
    }

    #[test]
    fn an_occurrence_with_no_range_at_all_yields_nothing() {
        assert!(occurrence_span(&Occurrence::new()).is_none());
        assert!(occurrence_enclosing_span(&Occurrence::new()).is_none());
    }
}
