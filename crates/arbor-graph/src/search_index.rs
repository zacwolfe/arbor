//! Symbol search over names, identifier tokens, and documentation.
//!
//! Three layers, each answering a different kind of question:
//!
//! 1. **Exact / substring** — `validate` finds `validate_user`. Cheap, precise,
//!    and what `arbor query` has always done.
//! 2. **Token** — identifiers are split into words, so `user profile` finds
//!    `getUserProfile` even though neither is a substring of the other.
//! 3. **Concept** — tokens expand through a curated lexicon, so `login` finds
//!    `get_authenticated`. This is the layer that makes "show me the auth code"
//!    work on a codebase that never uses the word "auth".
//!
//! Every layer is deterministic and offline. Results carry the match kind so a
//! caller can tell an exact hit from a concept guess rather than being handed
//! an unexplained ranking.

use crate::graph::NodeId;
use crate::lexicon::{stem, tokenize_identifier, Lexicon};
use std::collections::{HashMap, HashSet};

/// Minimum n-gram length for indexing.
const MIN_NGRAM_LEN: usize = 2;

/// Maximum n-gram length for indexing.
const MAX_NGRAM_LEN: usize = 4;

/// How a symbol matched a query, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    /// The query is the whole name.
    Exact,
    /// The query appears verbatim inside the name.
    Substring,
    /// Every query word appears among the name's words.
    AllTokens,
    /// Some query words appear among the name's words.
    SomeTokens,
    /// Query words are lexically related to the name's words.
    Concept,
    /// Query words appear in the docstring, signature, or file path.
    Documentation,
}

impl MatchKind {
    /// Ranking weight in `[0.0, 1.0]`.
    pub fn score(self) -> f32 {
        match self {
            MatchKind::Exact => 1.0,
            MatchKind::Substring => 0.85,
            MatchKind::AllTokens => 0.80,
            MatchKind::SomeTokens => 0.55,
            MatchKind::Concept => 0.45,
            MatchKind::Documentation => 0.30,
        }
    }

    /// Short human-readable label, for explaining a result.
    pub fn label(self) -> &'static str {
        match self {
            MatchKind::Exact => "exact name",
            MatchKind::Substring => "name contains query",
            MatchKind::AllTokens => "all query words in name",
            MatchKind::SomeTokens => "some query words in name",
            MatchKind::Concept => "related concept",
            MatchKind::Documentation => "mentioned in docs or path",
        }
    }

    /// Whether this match rests on the symbol's own name rather than inference.
    pub fn is_literal(self) -> bool {
        matches!(self, MatchKind::Exact | MatchKind::Substring)
    }
}

/// A ranked search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub id: NodeId,
    pub kind: MatchKind,
    pub score: f32,
}

/// An inverted index over symbol names, tokens, and documentation.
#[derive(Debug, Default, Clone)]
pub struct SearchIndex {
    /// Lowercased full name → nodes.
    exact_index: HashMap<String, Vec<NodeId>>,
    /// Lowercased n-gram → nodes.
    ngram_index: HashMap<String, HashSet<NodeId>>,
    /// Stemmed identifier token → nodes.
    token_index: HashMap<String, HashSet<NodeId>>,
    /// Stemmed token from docstring / signature / path → nodes.
    doc_token_index: HashMap<String, HashSet<NodeId>>,
    /// Node → its lowercased name.
    ///
    /// Exists so substring verification is a direct lookup. The previous
    /// implementation re-scanned the whole name index once per candidate,
    /// making `search` O(candidates × distinct names) — slower than the linear
    /// scan its documentation claimed to replace.
    name_by_id: HashMap<NodeId, String>,
    lexicon: Lexicon,
}

impl SearchIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Indexes a symbol name.
    pub fn insert(&mut self, name: &str, id: NodeId) {
        let lower = name.to_lowercase();

        self.exact_index.entry(lower.clone()).or_default().push(id);
        self.name_by_id.insert(id, lower.clone());

        for ngram in generate_ngrams(&lower) {
            self.ngram_index.entry(ngram).or_default().insert(id);
        }

        for token in tokenize_identifier(name) {
            self.token_index.entry(stem(&token)).or_default().insert(id);
        }
    }

    /// Indexes supporting text: a docstring, signature, or file path.
    ///
    /// This text is already parsed into every `CodeNode` and was previously
    /// discarded at search time, even though a function documented as
    /// "validates the user's login credentials" is exactly what someone
    /// searching for "login" wants.
    pub fn insert_documentation(&mut self, text: &str, id: NodeId) {
        for token in tokenize_identifier(text) {
            self.doc_token_index
                .entry(stem(&token))
                .or_default()
                .insert(id);
        }
    }

    /// Removes a symbol name from the index.
    pub fn remove(&mut self, name: &str, id: NodeId) {
        let lower = name.to_lowercase();

        if let Some(ids) = self.exact_index.get_mut(&lower) {
            ids.retain(|&x| x != id);
            if ids.is_empty() {
                self.exact_index.remove(&lower);
            }
        }
        self.name_by_id.remove(&id);

        for ngram in generate_ngrams(&lower) {
            if let Some(ids) = self.ngram_index.get_mut(&ngram) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.ngram_index.remove(&ngram);
                }
            }
        }

        for token in tokenize_identifier(name) {
            let key = stem(&token);
            if let Some(ids) = self.token_index.get_mut(&key) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.token_index.remove(&key);
                }
            }
        }

        // Documentation tokens are not keyed by name, so the node has to be
        // swept out of every posting list it appears in.
        self.doc_token_index.retain(|_, ids| {
            ids.remove(&id);
            !ids.is_empty()
        });
    }

    /// Literal substring search. Unchanged semantics from earlier versions.
    ///
    /// Returns node IDs sorted for deterministic output.
    pub fn search(&self, query: &str) -> Vec<NodeId> {
        let mut ids: Vec<NodeId> = self
            .search_ranked(query)
            .into_iter()
            .filter(|hit| hit.kind.is_literal())
            .map(|hit| hit.id)
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Full search across names, tokens, concepts, and documentation.
    ///
    /// Results are ordered by score, then by node index so ties are stable.
    /// Each node appears once, under its strongest match kind.
    pub fn search_ranked(&self, query: &str) -> Vec<SearchHit> {
        let query_lower = query.trim().to_lowercase();
        if query_lower.is_empty() {
            return Vec::new();
        }

        // Strongest kind seen per node.
        let mut best: HashMap<NodeId, MatchKind> = HashMap::new();
        let record = |id: NodeId, kind: MatchKind, best: &mut HashMap<NodeId, MatchKind>| {
            best.entry(id)
                .and_modify(|existing| {
                    if kind < *existing {
                        *existing = kind;
                    }
                })
                .or_insert(kind);
        };

        // ── Layer 1: exact and substring ─────────────────────────────────────
        if let Some(ids) = self.exact_index.get(&query_lower) {
            for &id in ids {
                record(id, MatchKind::Exact, &mut best);
            }
        }

        for id in self.substring_candidates(&query_lower) {
            record(id, MatchKind::Substring, &mut best);
        }

        // ── Layer 2 and 3: tokens, then concepts ─────────────────────────────
        let query_tokens: Vec<String> = tokenize_identifier(&query_lower);

        if !query_tokens.is_empty() {
            let stems: Vec<String> = query_tokens.iter().map(|t| stem(t)).collect();

            // Count how many distinct query words each node matched.
            let mut token_hits: HashMap<NodeId, usize> = HashMap::new();
            for s in &stems {
                if let Some(ids) = self.token_index.get(s) {
                    for &id in ids {
                        *token_hits.entry(id).or_insert(0) += 1;
                    }
                }
            }
            for (id, hits) in token_hits {
                let kind = if hits == stems.len() {
                    MatchKind::AllTokens
                } else {
                    MatchKind::SomeTokens
                };
                record(id, kind, &mut best);
            }

            // Concept expansion: `login` also looks for `authenticate`,
            // `session`, `credential`, and the rest of its cluster.
            for token in &query_tokens {
                for related in self.lexicon.expand(token) {
                    if &related == token {
                        continue;
                    }
                    if let Some(ids) = self.token_index.get(&stem(&related)) {
                        for &id in ids {
                            record(id, MatchKind::Concept, &mut best);
                        }
                    }
                }
            }

            // ── Layer 4: documentation, signatures, paths ────────────────────
            for s in &stems {
                if let Some(ids) = self.doc_token_index.get(s) {
                    for &id in ids {
                        record(id, MatchKind::Documentation, &mut best);
                    }
                }
            }
        }

        let mut hits: Vec<SearchHit> = best
            .into_iter()
            .map(|(id, kind)| SearchHit {
                id,
                kind,
                score: kind.score(),
            })
            .collect();

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.id.index().cmp(&b.id.index()))
        });

        hits
    }

    /// Nodes whose name contains the query as a literal substring.
    fn substring_candidates(&self, query_lower: &str) -> Vec<NodeId> {
        if query_lower.len() < MIN_NGRAM_LEN {
            // Too short to n-gram; fall back to prefix matching.
            let mut out: Vec<NodeId> = self
                .exact_index
                .iter()
                .filter(|(name, _)| name.starts_with(query_lower))
                .flat_map(|(_, ids)| ids.iter().copied())
                .collect();
            out.sort();
            out.dedup();
            return out;
        }

        let query_ngrams = generate_ngrams(query_lower);
        if query_ngrams.is_empty() {
            return Vec::new();
        }

        let mut candidates: Option<HashSet<NodeId>> = None;
        for ngram in &query_ngrams {
            let Some(ids) = self.ngram_index.get(ngram) else {
                // A query n-gram nothing contains means no substring match.
                return Vec::new();
            };
            match &mut candidates {
                None => candidates = Some(ids.clone()),
                Some(c) => c.retain(|id| ids.contains(id)),
            }
        }

        // n-gram intersection over-matches (the grams can appear out of order),
        // so verify against the real name — now an O(1) lookup per candidate.
        let mut out: Vec<NodeId> = candidates
            .unwrap_or_default()
            .into_iter()
            .filter(|id| {
                self.name_by_id
                    .get(id)
                    .is_some_and(|name| name.contains(query_lower))
            })
            .collect();
        out.sort();
        out
    }

    /// Number of unique names indexed.
    pub fn len(&self) -> usize {
        self.exact_index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.exact_index.is_empty()
    }
}

/// Generates all n-grams of length `MIN_NGRAM_LEN..=MAX_NGRAM_LEN`.
fn generate_ngrams(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut ngrams = Vec::new();

    for n in MIN_NGRAM_LEN..=MAX_NGRAM_LEN {
        if chars.len() >= n {
            for i in 0..=(chars.len() - n) {
                ngrams.push(chars[i..i + n].iter().collect());
            }
        }
    }

    ngrams
}

#[cfg(test)]
mod tests {
    use super::*;
    use petgraph::graph::NodeIndex;

    fn node_id(n: u32) -> NodeId {
        NodeIndex::new(n as usize)
    }

    fn kind_of(hits: &[SearchHit], id: NodeId) -> Option<MatchKind> {
        hits.iter().find(|h| h.id == id).map(|h| h.kind)
    }

    // ── Literal search: unchanged behaviour ──────────────────────────────────

    #[test]
    fn test_insert_and_search_exact() {
        let mut index = SearchIndex::new();
        index.insert("validate_user", node_id(0));
        index.insert("validate_email", node_id(1));
        index.insert("send_email", node_id(2));

        assert_eq!(index.search("validate_user"), vec![node_id(0)]);
    }

    #[test]
    fn test_search_substring() {
        let mut index = SearchIndex::new();
        index.insert("validate_user", node_id(0));
        index.insert("validate_email", node_id(1));
        index.insert("send_email", node_id(2));

        let results = index.search("validate");
        assert!(results.contains(&node_id(0)));
        assert!(results.contains(&node_id(1)));
        assert!(!results.contains(&node_id(2)));
    }

    #[test]
    fn test_search_case_insensitive() {
        let mut index = SearchIndex::new();
        index.insert("ValidateUser", node_id(0));

        assert_eq!(index.search("validateuser"), vec![node_id(0)]);
        assert_eq!(index.search("VALIDATEUSER"), vec![node_id(0)]);
    }

    #[test]
    fn test_search_middle_substring() {
        let mut index = SearchIndex::new();
        index.insert("get_user_profile", node_id(0));

        assert_eq!(index.search("user"), vec![node_id(0)]);
        assert_eq!(index.search("_user_"), vec![node_id(0)]);
    }

    #[test]
    fn test_remove_from_index() {
        let mut index = SearchIndex::new();
        index.insert("foo", node_id(0));
        index.insert("foobar", node_id(1));

        index.remove("foo", node_id(0));

        let results = index.search("foo");
        assert!(!results.contains(&node_id(0)));
        assert!(results.contains(&node_id(1)));
    }

    #[test]
    fn test_search_no_match() {
        let mut index = SearchIndex::new();
        index.insert("hello", node_id(0));
        assert!(index.search("world").is_empty());
    }

    #[test]
    fn test_short_query() {
        let mut index = SearchIndex::new();
        index.insert("ab", node_id(0));
        index.insert("abc", node_id(1));
        index.insert("xyz", node_id(2));

        let results = index.search("a");
        assert!(results.contains(&node_id(0)));
        assert!(results.contains(&node_id(1)));
        assert!(!results.contains(&node_id(2)));
    }

    #[test]
    fn test_index_len_and_is_empty() {
        let mut index = SearchIndex::new();
        assert!(index.is_empty());
        index.insert("foo", node_id(0));
        index.insert("bar", node_id(1));
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn test_duplicate_name_different_ids() {
        let mut index = SearchIndex::new();
        index.insert("process", node_id(0));
        index.insert("process", node_id(1));

        let results = index.search("process");
        assert!(results.contains(&node_id(0)));
        assert!(results.contains(&node_id(1)));
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn test_remove_nonexistent_does_not_panic() {
        let mut index = SearchIndex::new();
        index.remove("nonexistent", node_id(99));
        assert!(index.is_empty());
    }

    #[test]
    fn test_search_empty_query() {
        let mut index = SearchIndex::new();
        index.insert("hello", node_id(0));
        assert!(index.search("").is_empty());
    }

    // ── The naming gap ───────────────────────────────────────────────────────

    #[test]
    fn login_finds_get_authenticated() {
        // The motivating case. These share no substring whatsoever, so literal
        // search returns nothing at all.
        let mut index = SearchIndex::new();
        index.insert("get_authenticated", node_id(0));
        index.insert("render_chart", node_id(1));

        assert!(
            index.search("login").is_empty(),
            "literal search genuinely cannot find this"
        );

        let hits = index.search_ranked("login");
        assert_eq!(kind_of(&hits, node_id(0)), Some(MatchKind::Concept));
        assert_eq!(kind_of(&hits, node_id(1)), None);
    }

    #[test]
    fn security_query_reaches_the_auth_surface() {
        let mut index = SearchIndex::new();
        index.insert("verifyJwt", node_id(0));
        index.insert("hashPassword", node_id(1));
        index.insert("renderFooter", node_id(2));

        let hits = index.search_ranked("credentials");
        let found: Vec<NodeId> = hits.iter().map(|h| h.id).collect();

        assert!(found.contains(&node_id(0)));
        assert!(found.contains(&node_id(1)));
        assert!(!found.contains(&node_id(2)));
    }

    #[test]
    fn token_match_beats_concept_match() {
        let mut index = SearchIndex::new();
        index.insert("getUserProfile", node_id(0)); // literal token match
        index.insert("get_authenticated", node_id(1)); // concept only

        let hits = index.search_ranked("user");
        assert_eq!(hits.first().map(|h| h.id), Some(node_id(0)));
        assert!(kind_of(&hits, node_id(0)).unwrap() < MatchKind::Concept);
    }

    #[test]
    fn multi_word_query_prefers_full_coverage() {
        let mut index = SearchIndex::new();
        index.insert("getUserProfile", node_id(0));
        index.insert("getUserSettings", node_id(1));
        index.insert("deleteProfile", node_id(2));

        let hits = index.search_ranked("user profile");
        assert_eq!(kind_of(&hits, node_id(0)), Some(MatchKind::AllTokens));
        assert_eq!(kind_of(&hits, node_id(1)), Some(MatchKind::SomeTokens));
        assert_eq!(hits.first().map(|h| h.id), Some(node_id(0)));
    }

    #[test]
    fn documentation_is_searchable() {
        let mut index = SearchIndex::new();
        index.insert("checkAccess", node_id(0));
        index.insert_documentation(
            "Validates the user's login credentials before granting access.",
            node_id(0),
        );
        index.insert("renderChart", node_id(1));

        // "login" appears only in the docstring.
        let hits = index.search_ranked("login");
        assert!(hits.iter().any(|h| h.id == node_id(0)));
        assert!(!hits.iter().any(|h| h.id == node_id(1)));
    }

    #[test]
    fn results_are_deterministic_and_ranked() {
        let mut index = SearchIndex::new();
        for i in 0..20u32 {
            index.insert(&format!("authHandler{i}"), node_id(i));
        }

        let a = index.search_ranked("auth");
        let b = index.search_ranked("auth");
        assert_eq!(a, b, "repeat queries must return identical order");

        for pair in a.windows(2) {
            assert!(pair[0].score >= pair[1].score, "hits must be score-ordered");
        }
    }

    #[test]
    fn each_node_appears_once_under_its_best_kind() {
        let mut index = SearchIndex::new();
        // Matches as substring *and* token *and* concept.
        index.insert("auth", node_id(0));
        index.insert_documentation("auth auth auth", node_id(0));

        let hits = index.search_ranked("auth");
        assert_eq!(hits.iter().filter(|h| h.id == node_id(0)).count(), 1);
        assert_eq!(hits[0].kind, MatchKind::Exact);
    }

    #[test]
    fn unrelated_query_returns_nothing() {
        let mut index = SearchIndex::new();
        index.insert("renderChart", node_id(0));
        assert!(index.search_ranked("payment").is_empty());
    }

    #[test]
    fn match_kind_scores_are_ordered() {
        assert!(MatchKind::Exact.score() > MatchKind::Substring.score());
        assert!(MatchKind::Substring.score() > MatchKind::AllTokens.score());
        assert!(MatchKind::AllTokens.score() > MatchKind::SomeTokens.score());
        assert!(MatchKind::SomeTokens.score() > MatchKind::Concept.score());
        assert!(MatchKind::Concept.score() > MatchKind::Documentation.score());
        assert!(MatchKind::Exact.is_literal() && !MatchKind::Concept.is_literal());
    }
}
