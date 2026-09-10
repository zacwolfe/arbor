//! Translating SCIP symbol strings into Arbor's node vocabulary.
//!
//! A SCIP symbol is a single string carrying the whole path to a definition:
//!
//! ```text
//! semanticdb maven . . com/example/UserService#validate().
//! ^scheme    ^mgr  ^ ^ ^descriptors
//! ```
//!
//! The descriptor suffix (`/` package, `#` type, `.` term, `().` method) is
//! what tells us the *kind* of the thing without any guessing — the reason a
//! compiler-produced index beats pattern matching over source text.
//!
//! The grammar is identical across every indexer. Two things are not, and
//! [`SymbolStyle`] carries exactly those: how a language spells the separator
//! between scopes, and what an indexer calls a constructor when it does not
//! classify one. Everything else here is language-neutral.

use arbor_core::NodeKind;
use protobuf::Enum;
use scip::symbol::parse_symbol;
use scip::types::{
    descriptor::Suffix, symbol_information, Descriptor, Language, SymbolInformation,
};

/// What a SCIP symbol string means in Arbor's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolFacts {
    /// Dotted fully-qualified name, e.g. `com.example.UserService.validate`.
    ///
    /// Overloads carry their SCIP disambiguator (`...validate+1`) so that two
    /// methods with the same name produce two distinct nodes. Collapsing them
    /// would merge unrelated bodies — and their callees — into one vertex.
    pub qualified_name: String,

    /// The last descriptor's own name, e.g. `validate`.
    pub simple_name: String,

    /// Kind implied by the descriptor suffix, before any refinement from
    /// [`refine_kind`].
    pub kind: NodeKind,
}

/// The language-shaped part of reading a SCIP symbol.
///
/// Deliberately data rather than a trait: the variation between languages here
/// is two values, and expressing two values as polymorphism is how a 50-line
/// change becomes a 500-line one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolStyle {
    /// Joins descriptor names into a qualified name — `.` for most languages,
    /// `::` for Rust and C++.
    pub separator: &'static str,

    /// What this language's indexer calls a constructor when it emits one as an
    /// ordinary method.
    ///
    /// Only a fallback: [`refine_kind`] already promotes anything the indexer
    /// itself classifies as `Kind::Constructor`. This catches the indexers that
    /// do not, `scip-java` among them.
    pub ctor_names: &'static [&'static str],

    /// Whether an `impl` type descriptor should be replaced by the type it is an
    /// impl *of*, which the indexer keeps in the following type parameter.
    ///
    /// True for Rust only. Gated on the language rather than applied everywhere
    /// because a type legitimately named `impl` is possible elsewhere, and
    /// renaming it to its own type argument would be silently wrong.
    pub unwrap_impl_blocks: bool,
}

impl Default for SymbolStyle {
    /// Dot-separated, no constructor convention.
    ///
    /// The default is deliberately permissive: an unfamiliar indexer produces
    /// slightly plainer names rather than no graph at all.
    fn default() -> Self {
        Self {
            separator: ".",
            ctor_names: &[],
            unwrap_impl_blocks: false,
        }
    }
}

/// Decodes a raw `Document.language` field into the name it is supposed to
/// hold, or `None` if there is effectively no usable language to report.
///
/// The proto documents `language` as a *string name* (`scip.proto`: "The
/// `Language` enum contains the names of most common programming languages
/// ... typed as a string to permit any programming language"), but
/// `scip-php` writes the numeric enum value instead (`"19"` for
/// `Language::PHP`). An all-digit field is decoded through the enum rather
/// than trusted as a name — if the enum recognises it, its variant name
/// (`PHP`) is the true language; if it does not (a value from a newer schema,
/// or garbage), the field is treated the same as empty, i.e. absent, so a
/// caller's own scheme-based fallback gets a chance instead of a bare number
/// leaking into a report or a style lookup.
///
/// This is the single decode point: both the stats line and [`style_for`]
/// are meant to be fed the result of this function rather than the raw
/// field, so they cannot disagree about what a document's language is.
pub fn resolve_document_language(language: &str) -> Option<String> {
    if language.is_empty() {
        return None;
    }

    if language.bytes().all(|b| b.is_ascii_digit()) {
        return language
            .parse::<i32>()
            .ok()
            .and_then(Language::from_i32)
            .map(|lang| format!("{lang:?}"));
    }

    Some(language.to_string())
}

/// Picks a style for one document.
///
/// Keys on SCIP's own `Document.language` — already decoded by
/// [`resolve_document_language`], since a caller reading it straight off the
/// index would reintroduce the numeric-vs-name bug this module works around.
/// `sample_symbol` is a fallback for indexers that leave the language blank,
/// or (see below) put something in it Arbor doesn't recognise — the symbol's
/// scheme names the indexer, which implies the language just as well.
pub fn style_for(language: &str, sample_symbol: Option<&str>) -> SymbolStyle {
    let normalized = normalize(language);
    if !normalized.is_empty() {
        if let Some(style) = style_for_language(&normalized) {
            return style;
        }
        // A non-empty language we don't recognise is not treated as "use the
        // default and stop": that discards information Arbor might still be
        // able to recover. Fall through to the same scheme-based guess used
        // for a blank field, below.
    }

    let scheme = sample_symbol
        .and_then(|symbol| parse_symbol(symbol).ok())
        .map(|parsed| parsed.scheme)
        .unwrap_or_default();

    match scheme.as_str() {
        "semanticdb" => style_for_language("java"),
        "scip-typescript" => style_for_language("typescript"),
        "scip-python" => style_for_language("python"),
        "rust-analyzer" => style_for_language("rust"),
        "scip-clang" => style_for_language("cpp"),
        "scip-ruby" => style_for_language("ruby"),
        "scip-php" => style_for_language("php"),
        _ => None,
    }
    .unwrap_or_default()
}

/// The language an indexer covers, named from its scheme.
///
/// `scip-python` and some others leave `Document.language` empty, which left
/// Arbor reporting `languages: []` and printing an empty `()` after the tool
/// name. The scheme names the indexer, and an indexer covers a known language,
/// so this is derived rather than invented.
pub fn language_from_scheme(symbol: &str) -> Option<&'static str> {
    let scheme = parse_symbol(symbol).ok()?.scheme;
    match scheme.as_str() {
        "semanticdb" => Some("Java/Kotlin"),
        "scip-typescript" => Some("TypeScript"),
        "scip-python" => Some("Python"),
        "rust-analyzer" => Some("Rust"),
        "scip-clang" => Some("C/C++"),
        "scip-dotnet" => Some("C#"),
        "scip-go" => Some("Go"),
        "scip-ruby" => Some("Ruby"),
        "scip-php" => Some("PHP"),
        "scip-dart" => Some("Dart"),
        _ => None,
    }
}

/// Lowercases and drops punctuation so `C#`, `CSharp` and `c_sharp` agree.
fn normalize(language: &str) -> String {
    language
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// `None` means the name is not one of the languages Arbor knows a style
/// for — distinct from "known, and its style happens to be the default" —
/// so [`style_for`] can tell whether to keep looking (the scheme fallback)
/// or stop.
fn style_for_language(normalized: &str) -> Option<SymbolStyle> {
    match normalized {
        // `<init>` is how the JVM names a constructor; SCIP emits it as an
        // ordinary method, but callers reason about it as construction.
        "java" | "kotlin" | "scala" | "groovy" => Some(SymbolStyle {
            separator: ".",
            ctor_names: &["<init>"],
            unwrap_impl_blocks: false,
        }),
        "typescript" | "javascript" | "typescriptreact" | "javascriptreact" | "flow" => {
            Some(SymbolStyle {
                separator: ".",
                ctor_names: &["constructor"],
                unwrap_impl_blocks: false,
            })
        }
        "python" => Some(SymbolStyle {
            separator: ".",
            ctor_names: &["__init__"],
            unwrap_impl_blocks: false,
        }),
        "ruby" => Some(SymbolStyle {
            separator: ".",
            ctor_names: &["initialize"],
            unwrap_impl_blocks: false,
        }),
        "php" => Some(SymbolStyle {
            separator: ".",
            ctor_names: &["__construct"],
            unwrap_impl_blocks: false,
        }),
        // Rust's `new` is a convention, not a constructor — rust-analyzer emits
        // it as the plain associated function it is, and calling it a
        // constructor here would invent a distinction the language lacks.
        "rust" => Some(SymbolStyle {
            separator: "::",
            ctor_names: &[],
            unwrap_impl_blocks: true,
        }),
        "cpp" | "objectivecpp" | "cuda" => Some(SymbolStyle {
            separator: "::",
            ctor_names: &[],
            unwrap_impl_blocks: false,
        }),
        _ => None,
    }
}

/// Whether a symbol could ever be a graph vertex.
///
/// Cheaper than [`parse`] and independent of any language style, which is why
/// the reference pass uses it: filtering locals and parameters is the same
/// question in every language.
pub fn is_graph_symbol(symbol: &str) -> bool {
    vertex_descriptors(symbol).is_some()
}

/// The descriptor list of a symbol that could be a vertex, or `None`.
///
/// Local symbols are rejected outright: they are per-file and enormously
/// numerous, and admitting them would bury the real structure.
fn vertex_descriptors(symbol: &str) -> Option<Vec<Descriptor>> {
    if symbol.is_empty() || scip::symbol::is_local_symbol(symbol) {
        return None;
    }

    let descriptors = parse_symbol(symbol).ok()?.descriptors;
    let last = descriptors.last()?;
    // A suffix with no node kind (parameter, type parameter, metadata) is not a
    // vertex, so there is nothing to report.
    suffix_to_kind(
        last.suffix.enum_value().ok()?,
        &last.name,
        &descriptors,
        &SymbolStyle::default(),
    )?;

    Some(descriptors)
}

/// Parses a global SCIP symbol into node facts.
///
/// Returns `None` for symbols that should never become graph vertices:
/// locals, parameters, type parameters, metadata symbols, and anything whose
/// descriptor list we cannot read.
pub fn parse(symbol: &str, style: &SymbolStyle) -> Option<SymbolFacts> {
    let descriptors = vertex_descriptors(symbol)?;
    let last = descriptors.last()?;

    let kind = suffix_to_kind(
        last.suffix.enum_value().ok()?,
        &last.name,
        &descriptors,
        style,
    )?;

    let named = strip_file_path_prefix(&descriptors);
    let mut qualified_name = scope_names(named, style).join(style.separator);

    if !last.disambiguator.is_empty() {
        qualified_name.push_str(&last.disambiguator);
    }

    if qualified_name.is_empty() {
        return None;
    }

    Some(SymbolFacts {
        qualified_name,
        simple_name: last.name.clone(),
        kind,
    })
}

/// The scope names that make up a qualified name, in order.
///
/// Type parameters, parameters and metadata never name a scope, so they are
/// dropped — except when they are the only place the scope's real name is kept,
/// which is what [`SymbolStyle::unwrap_impl_blocks`] is about.
fn scope_names<'a>(descriptors: &'a [Descriptor], style: &SymbolStyle) -> Vec<&'a str> {
    let mut names = Vec::with_capacity(descriptors.len());

    for (position, descriptor) in descriptors.iter().enumerate() {
        let suffix = descriptor.suffix.enum_value();

        if style.unwrap_impl_blocks
            && matches!(suffix, Ok(Suffix::Type))
            && descriptor.name == "impl"
        {
            // `rust-analyzer` writes an inherent method as
            // `graph/impl#[ArborGraph]add_pinned_edges().` — the type is a
            // *type parameter* descriptor, and `impl` is the type. Taking the
            // descriptors at face value yields `graph::impl::add_pinned_edges`,
            // which names a keyword instead of the receiver. Substituting the
            // first type parameter gives `graph::ArborGraph::add_pinned_edges`,
            // which is how the path is actually written in Rust.
            //
            // A trait impl carries two — `impl#[`Vec<T, A>`][IntoIterator]` —
            // and the receiver is the first. The trait is recoverable from the
            // `is_implementation` relationships if it is ever wanted.
            if let Some(receiver) = descriptors
                .get(position + 1)
                .filter(|next| matches!(next.suffix.enum_value(), Ok(Suffix::TypeParameter)))
                .map(|next| next.name.as_str())
                .filter(|name| !name.is_empty())
            {
                names.push(receiver);
                continue;
            }
        }

        if matches!(
            suffix,
            Ok(Suffix::TypeParameter) | Ok(Suffix::Parameter) | Ok(Suffix::Meta)
        ) {
            continue;
        }

        if !descriptor.name.is_empty() {
            names.push(descriptor.name.as_str());
        }
    }

    names
}

/// Drops the leading namespace descriptors that spell out a source file path.
///
/// `scip-typescript` puts the file path in them — `src/`foo.ts`/Bar#baz().` —
/// where `scip-java` puts a package. Joining a path with `.` yields
/// `src.foo.ts.Bar.baz`, which reads as nonsense and matches nothing a user
/// would type. The path is already on [`arbor_core::CodeNode::file`], and
/// `CodeNode::compute_id` hashes that file in, so dropping it here costs no
/// uniqueness.
///
/// Detected by extension rather than by indexer, so it covers every indexer
/// that path-qualifies without needing to know which ones those are.
fn strip_file_path_prefix(descriptors: &[Descriptor]) -> &[Descriptor] {
    let file_at = descriptors.iter().rposition(|d| {
        matches!(
            d.suffix.enum_value(),
            Ok(Suffix::Namespace) | Ok(Suffix::Package)
        ) && looks_like_source_file(&d.name)
    });

    let Some(index) = file_at else {
        return descriptors;
    };

    match descriptors.get(index + 1..) {
        // The symbol is the file itself. Keep it, named after the file, rather
        // than returning nothing and dropping the module node entirely.
        Some([]) | None => &descriptors[index..=index],
        Some(rest) => rest,
    }
}

/// Whether a descriptor name is a source file name rather than a scope name.
fn looks_like_source_file(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && arbor_core::languages::is_supported(ext))
}

/// Whether the scope directly containing the last descriptor is a type.
///
/// Type parameters, parameters and metadata are skipped: `rust-analyzer` writes
/// an inherent method as `impl#[ArborGraph]method()`, so the descriptor
/// immediately before the method is a *type parameter* and the enclosing type is
/// one step further back.
fn encloses_a_type(descriptors: &[Descriptor]) -> bool {
    descriptors
        .iter()
        .rev()
        .skip(1)
        .filter(|d| {
            !matches!(
                d.suffix.enum_value(),
                Ok(Suffix::TypeParameter) | Ok(Suffix::Parameter) | Ok(Suffix::Meta)
            )
        })
        .map(|d| d.suffix.enum_value())
        .next()
        .is_some_and(|suffix| matches!(suffix, Ok(Suffix::Type)))
}

/// Maps a descriptor suffix to a node kind.
///
/// `None` means "not a graph vertex".
fn suffix_to_kind(
    suffix: Suffix,
    name: &str,
    descriptors: &[Descriptor],
    style: &SymbolStyle,
) -> Option<NodeKind> {
    match suffix {
        Suffix::Method if style.ctor_names.contains(&name) => Some(NodeKind::Constructor),
        // SCIP spells a free function and a method identically — both are `()`.
        // What separates them is the enclosing scope: a method hangs off a type,
        // a function off a namespace or package. Without this, every top-level
        // Python, Go and Rust function is reported as a `method`, which is both
        // wrong and makes ambiguous-name output unreadable.
        Suffix::Method if encloses_a_type(descriptors) => Some(NodeKind::Method),
        Suffix::Method => Some(NodeKind::Function),
        Suffix::Type => Some(NodeKind::Class),
        Suffix::Term => Some(NodeKind::Field),
        Suffix::Namespace | Suffix::Package => Some(NodeKind::Module),
        Suffix::Macro => Some(NodeKind::Function),
        Suffix::TypeParameter
        | Suffix::Parameter
        | Suffix::Meta
        | Suffix::Local
        | Suffix::UnspecifiedSuffix => None,
    }
}

/// Sharpens a descriptor-derived kind using the indexer's own classification.
///
/// The descriptor suffix cannot tell an interface from a class — both are `#`.
/// `SymbolInformation.kind` can, and getting this right matters: entry-point
/// detection and dispatch expansion both key off interface-ness.
pub fn refine_kind(fallback: NodeKind, info: Option<&SymbolInformation>) -> NodeKind {
    use symbol_information::Kind;

    let Some(kind) = info.and_then(|i| i.kind.enum_value().ok()) else {
        return fallback;
    };

    match kind {
        Kind::Interface | Kind::Protocol | Kind::Trait | Kind::TypeClass => NodeKind::Interface,
        Kind::Class | Kind::SingletonClass | Kind::Object => NodeKind::Class,
        Kind::Enum => NodeKind::Enum,
        Kind::Struct => NodeKind::Struct,
        Kind::TypeAlias => NodeKind::TypeAlias,
        Kind::Constructor => NodeKind::Constructor,
        Kind::Constant | Kind::EnumMember => NodeKind::Constant,
        Kind::Field | Kind::StaticField | Kind::Property | Kind::StaticProperty => NodeKind::Field,
        Kind::Variable | Kind::StaticVariable => NodeKind::Variable,
        Kind::Function | Kind::Macro => NodeKind::Function,
        Kind::Method
        | Kind::StaticMethod
        | Kind::AbstractMethod
        | Kind::TraitMethod
        | Kind::ProtocolMethod
        | Kind::PureVirtualMethod
        | Kind::SingletonMethod
        | Kind::Getter
        | Kind::Setter
        | Kind::Accessor => NodeKind::Method,
        Kind::Package | Kind::PackageObject | Kind::Namespace | Kind::Module => NodeKind::Module,
        // Everything else (parameters, primitives, language-specific exotica)
        // adds nothing over what the descriptor already told us.
        _ => fallback,
    }
}

/// Whether a symbol denotes something callable.
///
/// Used to decide whether a reference becomes a `Calls` edge or a weaker
/// `UsesType` / `References` edge.
pub fn is_callable(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Method | NodeKind::Function | NodeKind::Constructor
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jvm() -> SymbolStyle {
        style_for("Java", None)
    }

    #[test]
    fn parses_java_method_symbol() {
        let facts = parse(
            "semanticdb maven . . com/example/UserService#validate().",
            &jvm(),
        )
        .unwrap();
        assert_eq!(facts.qualified_name, "com.example.UserService.validate");
        assert_eq!(facts.simple_name, "validate");
        assert_eq!(facts.kind, NodeKind::Method);
    }

    #[test]
    fn parses_java_type_symbol() {
        let facts = parse("semanticdb maven . . com/example/UserService#", &jvm()).unwrap();
        assert_eq!(facts.qualified_name, "com.example.UserService");
        assert_eq!(facts.kind, NodeKind::Class);
    }

    #[test]
    fn parses_field_symbol() {
        let facts = parse("semanticdb maven . . com/example/UserService#repo.", &jvm()).unwrap();
        assert_eq!(facts.qualified_name, "com.example.UserService.repo");
        assert_eq!(facts.kind, NodeKind::Field);
    }

    #[test]
    fn constructor_recognised_from_init() {
        let facts = parse(
            "semanticdb maven . . com/example/UserService#`<init>`().",
            &jvm(),
        )
        .unwrap();
        assert_eq!(facts.kind, NodeKind::Constructor);
    }

    #[test]
    fn overloads_get_distinct_qualified_names() {
        let a = parse("semanticdb maven . . com/example/Svc#find().", &jvm()).unwrap();
        let b = parse("semanticdb maven . . com/example/Svc#find(+1).", &jvm()).unwrap();
        assert_ne!(a.qualified_name, b.qualified_name);
        assert_eq!(b.qualified_name, "com.example.Svc.find+1");
        // Both still answer to the same searchable simple name.
        assert_eq!(a.simple_name, b.simple_name);
    }

    #[test]
    fn local_symbols_are_rejected() {
        assert!(parse("local 12", &jvm()).is_none());
        assert!(!is_graph_symbol("local 12"));
    }

    #[test]
    fn empty_symbol_is_rejected() {
        assert!(parse("", &jvm()).is_none());
        assert!(!is_graph_symbol(""));
    }

    #[test]
    fn typescript_file_path_is_not_part_of_the_name() {
        // scip-typescript puts the source file's path in the leading namespace
        // descriptors. Keeping it would read `src.userService.ts.UserService.find`.
        let style = style_for("TypeScript", None);
        let facts = parse(
            "scip-typescript npm app 1.0.0 `src/userService.ts`/UserService#find().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.qualified_name, "UserService.find");
        assert_eq!(facts.kind, NodeKind::Method);
    }

    #[test]
    fn typescript_module_symbol_keeps_the_file_name() {
        // Stripping the path must not leave nothing: the file's own symbol is a
        // real module node.
        let style = style_for("TypeScript", None);
        let facts = parse(
            "scip-typescript npm app 1.0.0 `src/userService.ts`/",
            &style,
        )
        .unwrap();
        assert_eq!(facts.qualified_name, "src/userService.ts");
        assert_eq!(facts.kind, NodeKind::Module);
    }

    #[test]
    fn typescript_constructor_is_recognised() {
        let style = style_for("TypeScript", None);
        let facts = parse(
            "scip-typescript npm app 1.0.0 `src/svc.ts`/Svc#constructor().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.kind, NodeKind::Constructor);
    }

    #[test]
    fn python_keeps_its_module_path() {
        // Python's namespace descriptors are module names, not file names, and
        // the module path is part of how Python code is referred to.
        let style = style_for("Python", None);
        let facts = parse(
            "scip-python python app 1.0 app/svc/UserService#find().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.qualified_name, "app.svc.UserService.find");
    }

    #[test]
    fn a_module_level_function_is_not_a_method() {
        // SCIP spells both as `()`. Python's `def resolve_edge` at module level
        // was being reported as a method, which is wrong and makes an ambiguous
        // name impossible to read.
        let style = style_for("Python", None);
        let facts = parse(
            "scip-python python app 1.0 dedupe_edges/resolve_edge().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.kind, NodeKind::Function);
        assert_eq!(facts.qualified_name, "dedupe_edges.resolve_edge");
    }

    #[test]
    fn a_method_on_a_type_is_still_a_method() {
        let style = style_for("Python", None);
        let facts = parse(
            "scip-python python app 1.0 app/svc/UserService#find().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.kind, NodeKind::Method);
    }

    #[test]
    fn a_rust_inherent_method_survives_the_impl_indirection() {
        // The descriptor before the method is a type *parameter*, so a naive
        // "is my parent a type" check would call this a free function.
        let style = style_for("Rust", None);
        let facts = parse(
            "rust-analyzer cargo arbor 0.1.0 graph/impl#[ArborGraph]add_edge().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.kind, NodeKind::Method);
    }

    #[test]
    fn a_rust_free_function_is_a_function() {
        let style = style_for("Rust", None);
        let facts = parse(
            "rust-analyzer cargo arbor 0.1.0 graph/resolve_edges().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.kind, NodeKind::Function);
    }

    #[test]
    fn python_dunder_init_is_a_constructor() {
        let style = style_for("Python", None);
        let facts = parse(
            "scip-python python app 1.0 app/svc/User#__init__().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.kind, NodeKind::Constructor);
    }

    #[test]
    fn rust_impl_block_is_named_after_its_type() {
        // rust-analyzer writes the receiver as a type parameter of an `impl`
        // type descriptor. Taken literally that reads `graph::impl::add_edge`.
        let style = style_for("Rust", None);
        let facts = parse(
            "rust-analyzer cargo arbor-graph 3.0.0 graph/impl#[ArborGraph]add_pinned_edges().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.qualified_name, "graph::ArborGraph::add_pinned_edges");
    }

    #[test]
    fn rust_trait_impl_takes_the_receiver_not_the_trait() {
        let style = style_for("Rust", None);
        let facts = parse(
            "rust-analyzer cargo alloc . vec/impl#[`Vec<T, A>`][IntoIterator]into_iter().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.qualified_name, "vec::Vec<T, A>::into_iter");
    }

    #[test]
    fn impl_unwrapping_is_rust_only() {
        // A Java type really named `impl` must keep its name.
        let style = style_for("Java", None);
        let facts = parse("semanticdb maven . . com/example/impl#[T]get().", &style).unwrap();
        assert_eq!(facts.qualified_name, "com.example.impl.get");
    }

    #[test]
    fn rust_uses_path_separators() {
        let style = style_for("Rust", None);
        let facts = parse(
            "rust-analyzer cargo arbor 0.1.0 graph/ArborGraph#add_edge().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.qualified_name, "graph::ArborGraph::add_edge");
    }

    #[test]
    fn rust_new_is_not_a_constructor() {
        // `new` is a naming convention in Rust, not a language construct.
        let style = style_for("Rust", None);
        let facts = parse(
            "rust-analyzer cargo arbor 0.1.0 graph/ArborGraph#new().",
            &style,
        )
        .unwrap();
        assert_eq!(facts.kind, NodeKind::Method);
    }

    #[test]
    fn style_falls_back_to_the_scheme_when_language_is_blank() {
        let style = style_for("", Some("rust-analyzer cargo arbor 0.1.0 graph/Foo#bar()."));
        assert_eq!(style.separator, "::");

        let style = style_for("", Some("semanticdb maven . . com/example/Svc#find()."));
        assert_eq!(style.ctor_names, &["<init>"]);
    }

    #[test]
    fn unknown_language_still_produces_a_usable_name() {
        let style = style_for("Brainfuck", None);
        let facts = parse("scip-bf pkg . . a/b/C#d().", &style).unwrap();
        assert_eq!(facts.qualified_name, "a.b.C.d");
    }

    #[test]
    fn language_names_normalise() {
        assert_eq!(style_for("C#", None), style_for("csharp", None));
        assert_eq!(style_for("TypeScript", None), style_for("typescript", None));
    }

    #[test]
    fn refine_kind_promotes_interface() {
        let mut info = SymbolInformation::new();
        info.kind = symbol_information::Kind::Interface.into();
        assert_eq!(
            refine_kind(NodeKind::Class, Some(&info)),
            NodeKind::Interface
        );
    }

    #[test]
    fn refine_kind_falls_back_without_info() {
        assert_eq!(refine_kind(NodeKind::Class, None), NodeKind::Class);
    }

    #[test]
    fn callable_kinds() {
        assert!(is_callable(NodeKind::Method));
        assert!(is_callable(NodeKind::Constructor));
        assert!(!is_callable(NodeKind::Class));
    }

    #[test]
    fn resolve_document_language_decodes_the_numeric_enum_value() {
        // scip-php writes the numeric `Language` enum value (PHP = 19) into
        // `Document.language`, where the schema documents a string name.
        assert_eq!(resolve_document_language("19"), Some("PHP".to_string()));
    }

    #[test]
    fn resolve_document_language_treats_an_unknown_number_as_absent() {
        // A value the enum doesn't know — garbage, or a newer schema — must
        // not leak through as a fake language name; the caller's own
        // scheme-based fallback is meant to get a turn instead.
        assert_eq!(resolve_document_language("9999"), None);
    }

    #[test]
    fn resolve_document_language_passes_a_real_name_through() {
        assert_eq!(resolve_document_language("PHP"), Some("PHP".to_string()));
        assert_eq!(resolve_document_language("Java"), Some("Java".to_string()));
    }

    #[test]
    fn resolve_document_language_treats_empty_as_absent() {
        assert_eq!(resolve_document_language(""), None);
    }

    #[test]
    fn php_as_a_proper_string_still_gets_phps_style() {
        let style = style_for("PHP", None);
        assert_eq!(style.ctor_names, &["__construct"]);
        assert_eq!(style.separator, ".");
    }

    #[test]
    fn unrecognised_nonnumeric_language_falls_through_to_the_scheme() {
        // The root cause, independent of the numeric bug: previously any
        // non-empty, unrecognised language returned `SymbolStyle::default()`
        // immediately, discarding a scheme that could have named the real
        // language. `scip-php`'s numeric "19" is one way to trigger this, but
        // any indexer that writes something odd into `Document.language`
        // must degrade the same way.
        let style = style_for(
            "Brainfuck",
            Some("scip-php composer app 1.0.0 App/Foo#__construct()."),
        );
        assert_eq!(style.ctor_names, &["__construct"]);
    }
}
