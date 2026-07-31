//! Identifier tokenization and a code-domain synonym lexicon.
//!
//! Substring search cannot answer "where is login?" when the function is called
//! `get_authenticated` — there is no shared substring. Yet a reviewer asking
//! about auth means both, and so does anyone asking Arbor about a security
//! surface.
//!
//! This module closes that gap without embeddings: identifiers are split into
//! their real word tokens, and tokens are mapped onto a small hand-curated set
//! of concepts. It is deterministic, offline, auditable, and costs nothing to
//! run — the properties that make the rest of the engine worth trusting.
//!
//! Deliberately *not* a semantic model. It resolves vocabulary, not meaning. A
//! function named `handleThing` will not be found by "auth" no matter what it
//! does; for that, see structural role inference over the call graph.

use std::collections::{HashMap, HashSet};

/// Splits an identifier into lowercase word tokens.
///
/// Handles `snake_case`, `camelCase`, `PascalCase`, `kebab-case`, `SCREAMING_CASE`,
/// dotted paths, and acronym runs (`parseHTTPResponse` → `parse`, `http`, `response`).
///
/// Tokens shorter than two characters are dropped: single letters are loop
/// variables and type parameters, and indexing them matches everything.
pub fn tokenize_identifier(identifier: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = identifier.chars().collect();

    for i in 0..chars.len() {
        let c = chars[i];

        if !c.is_alphanumeric() {
            // `_`, `-`, `.`, `/`, space — all separators.
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }

        if c.is_uppercase() && !current.is_empty() {
            let prev = chars[i - 1];
            let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());

            // Break before a capital that starts a new word: either the
            // previous char was lowercase (`getUser`), or we are leaving an
            // acronym run into a new word (`HTTPResponse`).
            if prev.is_lowercase() || prev.is_numeric() || (prev.is_uppercase() && next_is_lower) {
                tokens.push(std::mem::take(&mut current));
            }
        }

        current.push(c.to_ascii_lowercase());
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens.retain(|t| t.len() >= 2);
    tokens
}

/// Reduces a token to a crude stem so `handlers`, `handler`, and `handling`
/// collapse together.
///
/// Intentionally conservative — over-stemming collapses unrelated words and
/// produces confident nonsense, which is worse than a missed match.
pub fn stem(token: &str) -> String {
    let t = token;

    for suffix in ["ing", "ers", "ed"] {
        if t.len() > suffix.len() + 3 && t.ends_with(suffix) {
            return t[..t.len() - suffix.len()].to_string();
        }
    }
    for suffix in ["es", "er", "s"] {
        if t.len() > suffix.len() + 2 && t.ends_with(suffix) {
            return t[..t.len() - suffix.len()].to_string();
        }
    }

    t.to_string()
}

/// Concept clusters over code vocabulary.
///
/// The first entry of each row is the canonical concept name; every entry maps
/// to it. Queries expand through this table in both directions, so searching
/// `login` reaches `authenticate`, `session`, and `credential`.
const CONCEPT_CLUSTERS: &[&[&str]] = &[
    &[
        "auth",
        "authenticate",
        "authenticated",
        "authentication",
        "authorize",
        "authorization",
        "login",
        "logout",
        "signin",
        "signout",
        "signup",
        "session",
        "credential",
        "credentials",
        "password",
        "passwd",
        "token",
        "jwt",
        "oauth",
        "sso",
        "saml",
        "identity",
        "principal",
        "bearer",
        "apikey",
        "mfa",
        "otp",
        "2fa",
    ],
    &[
        "crypto",
        "encrypt",
        "decrypt",
        "cipher",
        "hash",
        "digest",
        "sign",
        "signature",
        "verify",
        "hmac",
        "bcrypt",
        "scrypt",
        "argon",
        "pbkdf",
        "aes",
        "rsa",
        "nonce",
        "salt",
        "keypair",
    ],
    &[
        "secret",
        "secrets",
        "vault",
        "keystore",
        "credentialstore",
        "env",
        "environment",
        "config",
        "configuration",
        "settings",
    ],
    &[
        "payment",
        "pay",
        "billing",
        "invoice",
        "charge",
        "refund",
        "checkout",
        "subscription",
        "stripe",
        "paypal",
        "razorpay",
        "card",
        "price",
        "pricing",
        "plan",
        "wallet",
        "payout",
        "transaction",
    ],
    &[
        "database",
        "db",
        "sql",
        "query",
        "repository",
        "repo",
        "dao",
        "orm",
        "migration",
        "schema",
        "table",
        "insert",
        "update",
        "delete",
        "select",
        "transaction",
        "cursor",
        "connection",
        "pool",
    ],
    &[
        "network",
        "http",
        "https",
        "request",
        "response",
        "fetch",
        "client",
        "api",
        "endpoint",
        "route",
        "router",
        "handler",
        "controller",
        "middleware",
        "webhook",
        "rpc",
        "grpc",
        "socket",
        "url",
        "uri",
    ],
    &[
        "validate",
        "validation",
        "validator",
        "sanitize",
        "sanitizer",
        "escape",
        "check",
        "verify",
        "assert",
        "guard",
        "constraint",
        "schema",
    ],
    &[
        "user",
        "users",
        "account",
        "accounts",
        "profile",
        "member",
        "customer",
        "tenant",
        "organization",
        "org",
        "team",
        "role",
        "permission",
        "permissions",
        "acl",
        "policy",
    ],
    &[
        "file",
        "filesystem",
        "fs",
        "path",
        "directory",
        "dir",
        "upload",
        "download",
        "stream",
        "blob",
        "storage",
        "bucket",
    ],
    &[
        "log",
        "logger",
        "logging",
        "trace",
        "tracing",
        "audit",
        "metric",
        "metrics",
        "telemetry",
        "monitor",
        "monitoring",
        "observability",
    ],
    &[
        "cache",
        "caching",
        "memo",
        "memoize",
        "redis",
        "memcached",
        "ttl",
        "evict",
        "invalidate",
    ],
    &[
        "queue",
        "job",
        "jobs",
        "worker",
        "task",
        "scheduler",
        "cron",
        "background",
        "async",
        "batch",
        "consumer",
        "producer",
        "publish",
        "subscribe",
    ],
    &[
        "error",
        "errors",
        "exception",
        "panic",
        "fail",
        "failure",
        "fault",
        "retry",
        "fallback",
        "recover",
        "handle",
    ],
    &[
        "test",
        "tests",
        "spec",
        "fixture",
        "mock",
        "stub",
        "fake",
        "assert",
        "expect",
        "benchmark",
    ],
    &[
        "parse",
        "parser",
        "serialize",
        "serializer",
        "deserialize",
        "decode",
        "encode",
        "marshal",
        "unmarshal",
        "json",
        "yaml",
        "toml",
        "xml",
    ],
    &[
        "create",
        "new",
        "make",
        "build",
        "construct",
        "init",
        "initialize",
        "insert",
        "add",
        "register",
    ],
    &[
        "delete",
        "remove",
        "destroy",
        "drop",
        "purge",
        "erase",
        "clear",
        "unregister",
    ],
    &[
        "get", "fetch", "read", "load", "find", "lookup", "retrieve", "query", "list",
    ],
    &[
        "set", "write", "save", "store", "persist", "put", "commit", "flush",
    ],
];

/// Bidirectional token↔concept index over [`CONCEPT_CLUSTERS`].
#[derive(Debug, Clone)]
pub struct Lexicon {
    /// Token → the concepts it belongs to.
    token_to_concepts: HashMap<&'static str, Vec<&'static str>>,
    /// Concept → every token in it.
    concept_to_tokens: HashMap<&'static str, &'static [&'static str]>,
}

impl Default for Lexicon {
    fn default() -> Self {
        Self::new()
    }
}

impl Lexicon {
    pub fn new() -> Self {
        let mut token_to_concepts: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        let mut concept_to_tokens = HashMap::new();

        for cluster in CONCEPT_CLUSTERS {
            let Some(&concept) = cluster.first() else {
                continue;
            };
            concept_to_tokens.insert(concept, *cluster);
            for token in cluster.iter() {
                token_to_concepts.entry(*token).or_default().push(concept);
            }
        }

        Self {
            token_to_concepts,
            concept_to_tokens,
        }
    }

    /// Concepts a token belongs to, if any.
    pub fn concepts_for(&self, token: &str) -> &[&'static str] {
        self.token_to_concepts
            .get(token)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Every token related to the given one, including itself.
    ///
    /// `login` → `auth`, `authenticate`, `session`, `token`, …
    pub fn expand(&self, token: &str) -> HashSet<String> {
        let mut out = HashSet::new();
        out.insert(token.to_string());

        for concept in self.concepts_for(token) {
            if let Some(tokens) = self.concept_to_tokens.get(concept) {
                for t in tokens.iter() {
                    out.insert((*t).to_string());
                }
            }
        }

        out
    }

    /// Whether two tokens share a concept.
    pub fn related(&self, a: &str, b: &str) -> bool {
        if a == b {
            return true;
        }
        let a_concepts = self.concepts_for(a);
        !a_concepts.is_empty() && self.concepts_for(b).iter().any(|c| a_concepts.contains(c))
    }

    /// Number of concept clusters.
    pub fn concept_count(&self) -> usize {
        self.concept_to_tokens.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_snake_case() {
        assert_eq!(
            tokenize_identifier("get_authenticated_user"),
            ["get", "authenticated", "user"]
        );
    }

    #[test]
    fn splits_camel_and_pascal_case() {
        assert_eq!(
            tokenize_identifier("getUserProfile"),
            ["get", "user", "profile"]
        );
        assert_eq!(tokenize_identifier("UserService"), ["user", "service"]);
    }

    #[test]
    fn splits_acronym_runs() {
        assert_eq!(
            tokenize_identifier("parseHTTPResponse"),
            ["parse", "http", "response"]
        );
        assert_eq!(tokenize_identifier("HTTPServer"), ["http", "server"]);
    }

    #[test]
    fn splits_mixed_separators() {
        assert_eq!(
            tokenize_identifier("auth-service.verify_token"),
            ["auth", "service", "verify", "token"]
        );
        assert_eq!(
            tokenize_identifier("src/api/authHandler"),
            ["src", "api", "auth", "handler"]
        );
    }

    #[test]
    fn drops_single_character_tokens() {
        // Type parameters and loop variables match everything; indexing them
        // is pure noise.
        assert_eq!(tokenize_identifier("a_b_cd"), ["cd"]);
        assert_eq!(tokenize_identifier("mapToT"), ["map", "to"]);
        assert_eq!(tokenize_identifier("x"), Vec::<String>::new());
    }

    #[test]
    fn handles_screaming_case_and_digits() {
        assert_eq!(
            tokenize_identifier("MAX_RETRY_COUNT"),
            ["max", "retry", "count"]
        );
        assert_eq!(tokenize_identifier("oauth2Client"), ["oauth2", "client"]);
    }

    #[test]
    fn empty_and_degenerate_input() {
        assert!(tokenize_identifier("").is_empty());
        assert!(tokenize_identifier("_").is_empty());
        assert!(tokenize_identifier("___").is_empty());
    }

    #[test]
    fn stemming_is_conservative() {
        assert_eq!(stem("handlers"), "handl");
        assert_eq!(stem("handler"), "handl");
        assert_eq!(stem("tokens"), "token");
        // Too short to strip — refuses rather than mangling.
        assert_eq!(stem("as"), "as");
        assert_eq!(stem("api"), "api");
        assert_eq!(stem("les"), "les");
    }

    #[test]
    fn login_and_authenticated_share_a_concept() {
        // The exact case that motivated this module.
        let lex = Lexicon::new();
        assert!(lex.related("login", "authenticated"));
        assert!(lex.related("login", "session"));
        assert!(lex.related("signin", "jwt"));
    }

    #[test]
    fn expansion_reaches_synonyms() {
        let lex = Lexicon::new();
        let expanded = lex.expand("login");
        assert!(expanded.contains("authenticate"));
        assert!(expanded.contains("credential"));
        assert!(expanded.contains("login"));
    }

    #[test]
    fn unrelated_tokens_do_not_match() {
        let lex = Lexicon::new();
        assert!(!lex.related("login", "invoice"));
        assert!(!lex.related("cache", "password"));
        // An unknown token relates only to itself.
        assert!(!lex.related("wibble", "auth"));
        assert!(lex.related("wibble", "wibble"));
    }

    #[test]
    fn expansion_of_unknown_token_is_just_itself() {
        let lex = Lexicon::new();
        let expanded = lex.expand("zzzz");
        assert_eq!(expanded.len(), 1);
        assert!(expanded.contains("zzzz"));
    }

    #[test]
    fn every_cluster_has_a_canonical_head() {
        let lex = Lexicon::new();
        assert_eq!(lex.concept_count(), CONCEPT_CLUSTERS.len());
    }
}
