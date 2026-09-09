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

use arbor_core::NodeKind;
use scip::symbol::parse_symbol;
use scip::types::{descriptor::Suffix, symbol_information, SymbolInformation};

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

/// Parses a global SCIP symbol into node facts.
///
/// Returns `None` for symbols that should never become graph vertices:
/// locals, parameters, type parameters, metadata symbols, and anything whose
/// descriptor list we cannot read. Local symbols in particular are per-file
/// and enormously numerous; admitting them would bury the real structure.
pub fn parse(symbol: &str) -> Option<SymbolFacts> {
    if symbol.is_empty() || scip::symbol::is_local_symbol(symbol) {
        return None;
    }

    let parsed = parse_symbol(symbol).ok()?;
    let descriptors = parsed.descriptors;
    let last = descriptors.last()?;

    let kind = suffix_to_kind(last.suffix.enum_value().ok()?, &last.name)?;

    // Package descriptors join with `.`; a method or field hangs off its type
    // with `.` too, which is exactly Java/Kotlin/Scala FQN notation.
    let mut qualified_name = descriptors
        .iter()
        .filter(|d| {
            !matches!(
                d.suffix.enum_value(),
                Ok(Suffix::TypeParameter) | Ok(Suffix::Parameter) | Ok(Suffix::Meta)
            )
        })
        .map(|d| d.name.as_str())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(".");

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

/// Maps a descriptor suffix to a node kind.
///
/// `None` means "not a graph vertex".
fn suffix_to_kind(suffix: Suffix, name: &str) -> Option<NodeKind> {
    match suffix {
        // `<init>` is how the JVM names a constructor; SCIP emits it as an
        // ordinary method, but callers reason about it as construction.
        Suffix::Method if name == "<init>" => Some(NodeKind::Constructor),
        Suffix::Method => Some(NodeKind::Method),
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

    #[test]
    fn parses_java_method_symbol() {
        let facts = parse("semanticdb maven . . com/example/UserService#validate().").unwrap();
        assert_eq!(facts.qualified_name, "com.example.UserService.validate");
        assert_eq!(facts.simple_name, "validate");
        assert_eq!(facts.kind, NodeKind::Method);
    }

    #[test]
    fn parses_java_type_symbol() {
        let facts = parse("semanticdb maven . . com/example/UserService#").unwrap();
        assert_eq!(facts.qualified_name, "com.example.UserService");
        assert_eq!(facts.kind, NodeKind::Class);
    }

    #[test]
    fn parses_field_symbol() {
        let facts = parse("semanticdb maven . . com/example/UserService#repo.").unwrap();
        assert_eq!(facts.qualified_name, "com.example.UserService.repo");
        assert_eq!(facts.kind, NodeKind::Field);
    }

    #[test]
    fn constructor_recognised_from_init() {
        let facts = parse("semanticdb maven . . com/example/UserService#`<init>`().").unwrap();
        assert_eq!(facts.kind, NodeKind::Constructor);
    }

    #[test]
    fn overloads_get_distinct_qualified_names() {
        let a = parse("semanticdb maven . . com/example/Svc#find().").unwrap();
        let b = parse("semanticdb maven . . com/example/Svc#find(+1).").unwrap();
        assert_ne!(a.qualified_name, b.qualified_name);
        assert_eq!(b.qualified_name, "com.example.Svc.find+1");
        // Both still answer to the same searchable simple name.
        assert_eq!(a.simple_name, b.simple_name);
    }

    #[test]
    fn local_symbols_are_rejected() {
        assert!(parse("local 12").is_none());
    }

    #[test]
    fn empty_symbol_is_rejected() {
        assert!(parse("").is_none());
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
}
