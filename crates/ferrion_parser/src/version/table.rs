//! The feature table and the restricted identifiers (docs/design/parser-language-versions.md).
//! Every fact here was found by running javac 27 at each release on an input of Ferrion's own,
//! kept in `tests/version/` with javac's output.

use crate::JavacPhase;

/// When a feature is part of the language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// Final from this release on.
    Final(u32),
    /// A preview feature in the latest release and final in none. Ferrion never enables preview
    /// features, so its use is an error at every release.
    Preview,
}

/// What javac reports for a feature the release doesn't have: its diagnostic code (without
/// `compiler.err.`) and the argument that tells features apart: the feature's
/// `compiler.misc.feature.*` name, or for `expected`, the token expected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JavacDiagnostic {
    pub code: &'static str,
    pub argument: Option<&'static str>,
}

/// One row of the table.
#[derive(Clone, Copy, Debug)]
pub struct FeatureRow {
    pub feature: Feature,
    pub status: Status,
    /// When javac reports the feature at a release that doesn't have it.
    pub javac_phase: JavacPhase,
    /// What javac reports.
    pub javac: JavacDiagnostic,
    /// The feature's name in messages, such as "records".
    pub name: &'static str,
    /// Whether `name` takes a plural verb.
    pub plural: bool,
}

/// A language feature that needs a release later than 8, or a preview feature. Each variant's
/// documentation gives its anchor: the token the diagnostic's `anchor` points at, which is where
/// javac reports it.
///
/// javac reports each feature once per file, at its first use. Ferrion reports every use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Feature {
    /// A module declaration. Anchor: `module` (after any annotations and `open`).
    Modules,
    /// A `private` method in an interface. Anchor: the `(` of its parameters.
    PrivateInterfaceMethods,
    /// A variable or field as a try-with-resources resource: `try (r)`. Anchor: the token after
    /// the resource, the `;` or `)`.
    ResourceVariables,
    /// `<>` in a class instance creation with a class body. Anchor: the `<`.
    DiamondWithAnonymousClasses,
    /// `var` as a lambda parameter's type. Anchor: the parameter's name. Below 10, `var` is a
    /// type's name there, and the parameter gets a [restricted name](RestrictedName) warning
    /// instead.
    VarLambdaParameters,
    /// Anchor: `switch`.
    SwitchExpressions,
    /// `case ... ->`. Anchor: the `->`.
    SwitchRules,
    /// `case 1, 2`. Anchor: in a switch expression, the first `,`; in a switch statement, the
    /// second element (or `default`).
    MultipleCaseLabels,
    /// Anchor: the text block.
    TextBlocks,
    /// The `\s` escape sequence in a string or character literal. Anchor: the `s`.
    SpaceEscape,
    /// Anchor: `record`.
    Records,
    /// A type pattern in `instanceof`: `o instanceof String s`. Anchor: the binding's name.
    InstanceofPatterns,
    /// An enum declared in a block. Anchor: `enum`.
    LocalEnums,
    /// An interface or annotation interface declared in a block. Anchor: `interface`.
    LocalInterfaces,
    /// A static member of an inner class (a non-static member class, a local class, or an
    /// anonymous class): a static field that isn't a constant, a static method or initializer,
    /// or a member class, interface, or enum that is static. Anchor: a field's or method's name,
    /// an initializer's `static`, or a type's `class`, `interface`, or `enum`.
    StaticMembersOfInnerClasses,
    /// `sealed` or `non-sealed`. Anchor: `sealed`, or the `non` of `non-sealed`.
    SealedClasses,
    /// A `permits` clause. Below 17 `permits` isn't a keyword, and javac expects the class body
    /// there. Anchor: the end of the token before `permits`, as for javac's `'{' expected`.
    PermitsClauses,
    /// A pattern as a case label, or `default` after `case null,`. Anchor: the pattern's start,
    /// or the `default`.
    PatternsInSwitch,
    /// `case null`. Anchor: the `null`.
    CaseNull,
    /// Anchor: the record pattern's `(`.
    RecordPatterns,
    /// `_` as a local variable, a parameter of a lambda or `catch`, or a pattern variable, and
    /// `_` as a nested pattern. Anchor: the `_`, or the end of a nested `_` pattern. At 8, `_`
    /// as a variable is an identifier with a warning instead.
    UnnamedVariables,
    /// `import module m;`. Anchor: `module`.
    ModuleImports,
    /// Methods and fields at the top level. Anchor: the first of them, after its modifiers.
    ImplicitClasses,
    /// Statements before `super(...)` or `this(...)` in a constructor. Anchor: the call's `(`.
    FlexibleConstructorBodies,
    /// A primitive type in a top-level pattern, or after `instanceof`: `o instanceof int i`,
    /// `case int i`. Anchor: the type. Primitive types decided by the selector's or operand's
    /// type (a `switch` on a `long`) are left to semantic analysis.
    PrimitivePatterns,
}

const fn not_supported(feature: &'static str, plural: bool) -> JavacDiagnostic {
    JavacDiagnostic {
        code: if plural {
            "feature.not.supported.in.source.plural"
        } else {
            "feature.not.supported.in.source"
        },
        argument: Some(feature),
    }
}

const fn code(code: &'static str) -> JavacDiagnostic {
    JavacDiagnostic {
        code,
        argument: None,
    }
}

const fn row(
    feature: Feature,
    status: Status,
    javac_phase: JavacPhase,
    javac: JavacDiagnostic,
    name: &'static str,
    plural: bool,
) -> FeatureRow {
    FeatureRow {
        feature,
        status,
        javac_phase,
        javac,
        name,
        plural,
    }
}

use Feature as F;
use JavacPhase::{Later, Parse};
use Status::{Final, Preview};

/// The table, in the order of [`Feature`]'s variants, which is the order of the releases that
/// made them final.
pub const FEATURES: [FeatureRow; 25] = [
    row(
        F::Modules,
        Final(9),
        Parse,
        not_supported("modules", true),
        "modules",
        true,
    ),
    row(
        F::PrivateInterfaceMethods,
        Final(9),
        Parse,
        not_supported("private.intf.methods", true),
        "private interface methods",
        true,
    ),
    row(
        F::ResourceVariables,
        Final(9),
        Parse,
        not_supported("var.in.try.with.resources", true),
        "variables as try-with-resources resources",
        true,
    ),
    row(
        F::DiamondWithAnonymousClasses,
        Final(9),
        Later,
        JavacDiagnostic {
            code: "cant.apply.diamond.1",
            argument: Some("diamond.and.anon.class"),
        },
        "'<>' with anonymous classes",
        false,
    ),
    row(
        F::VarLambdaParameters,
        Final(11),
        Parse,
        not_supported("var.syntax.in.implicit.lambda", true),
        "'var' in lambda parameters",
        false,
    ),
    row(
        F::SwitchExpressions,
        Final(14),
        Parse,
        not_supported("switch.expressions", true),
        "switch expressions",
        true,
    ),
    row(
        F::SwitchRules,
        Final(14),
        Parse,
        not_supported("switch.rules", true),
        "switch rules ('case ->')",
        true,
    ),
    row(
        F::MultipleCaseLabels,
        Final(14),
        Parse,
        not_supported("multiple.case.labels", true),
        "multiple case labels",
        true,
    ),
    row(
        F::TextBlocks,
        Final(15),
        Parse,
        not_supported("text.blocks", true),
        "text blocks",
        true,
    ),
    row(
        F::SpaceEscape,
        Final(15),
        Parse,
        not_supported("text.blocks", true),
        "the '\\s' escape sequence",
        false,
    ),
    row(
        F::Records,
        Final(16),
        Parse,
        not_supported("records", true),
        "records",
        true,
    ),
    row(
        F::InstanceofPatterns,
        Final(16),
        Parse,
        not_supported("pattern.matching.instanceof", false),
        "patterns in 'instanceof'",
        true,
    ),
    row(
        F::LocalEnums,
        Final(16),
        Parse,
        code("local.enum"),
        "local enums",
        true,
    ),
    row(
        F::LocalInterfaces,
        Final(16),
        Later,
        code("intf.not.allowed.here"),
        "local interfaces",
        true,
    ),
    row(
        F::StaticMembersOfInnerClasses,
        Final(16),
        Later,
        code("icls.cant.have.static.decl"),
        "static members of inner classes",
        true,
    ),
    row(
        F::SealedClasses,
        Final(17),
        Parse,
        not_supported("sealed.classes", true),
        "sealed classes",
        true,
    ),
    row(
        F::PermitsClauses,
        Final(17),
        Parse,
        JavacDiagnostic {
            code: "expected",
            argument: Some("'{'"),
        },
        "'permits' clauses",
        true,
    ),
    row(
        F::PatternsInSwitch,
        Final(21),
        Parse,
        not_supported("pattern.switch", true),
        "patterns in switch",
        true,
    ),
    row(
        F::CaseNull,
        Final(21),
        Later,
        not_supported("case.null", false),
        "'case null'",
        false,
    ),
    row(
        F::RecordPatterns,
        Final(21),
        Parse,
        not_supported("deconstruction.patterns", true),
        "record patterns",
        true,
    ),
    row(
        F::UnnamedVariables,
        Final(22),
        Parse,
        not_supported("unnamed.variables", true),
        "unnamed variables ('_')",
        true,
    ),
    row(
        F::ModuleImports,
        Final(25),
        Parse,
        not_supported("module.imports", true),
        "module imports",
        true,
    ),
    row(
        F::ImplicitClasses,
        Final(25),
        Parse,
        not_supported("implicit.classes", true),
        "top-level methods and fields (implicitly declared classes)",
        true,
    ),
    row(
        F::FlexibleConstructorBodies,
        Final(25),
        Later,
        not_supported("flexible.constructors", false),
        "statements before 'super()' or 'this()'",
        true,
    ),
    row(
        F::PrimitivePatterns,
        Preview,
        Later,
        JavacDiagnostic {
            code: "preview.feature.disabled.plural",
            argument: Some("primitive.patterns"),
        },
        "primitive types in patterns",
        true,
    ),
];

impl Feature {
    /// Every feature, in the table's order.
    pub fn all() -> impl Iterator<Item = Feature> {
        FEATURES.iter().map(|row| row.feature)
    }

    pub fn row(self) -> &'static FeatureRow {
        &FEATURES[self as usize]
    }

    pub fn status(self) -> Status {
        self.row().status
    }

    pub fn javac_phase(self) -> JavacPhase {
        self.row().javac_phase
    }

    /// Whether `release` has the feature. Preview features are in no release.
    pub fn is_in(self, release: u32) -> bool {
        match self.status() {
            Final(first) => release >= first,
            Preview => false,
        }
    }

    /// The message for a use at `release`, which doesn't have the feature: "records require Java
    /// 16+ (the target is Java 11)", or "primitive types in patterns are a preview feature, not
    /// supported".
    pub fn message(self, release: u32) -> String {
        let row = self.row();
        match row.status {
            Final(first) => {
                let verb = if row.plural { "require" } else { "requires" };
                format!(
                    "{} {verb} Java {first}+ (the target is Java {release})",
                    row.name
                )
            }
            Preview => {
                let verb = if row.plural { "are" } else { "is" };
                format!("{} {verb} a preview feature, not supported", row.name)
            }
        }
    }

    /// The feature's name in snake case, which names its test input in `tests/version/`.
    pub fn id(self) -> String {
        let mut id = String::new();
        for (i, c) in format!("{self:?}").chars().enumerate() {
            if c.is_ascii_uppercase() {
                if i > 0 {
                    id.push('_');
                }
                id.push(c.to_ascii_lowercase());
            } else {
                id.push(c);
            }
        }
        id
    }
}

/// An identifier that a release restricted: from then on it can't name a type, and javac warns
/// about such uses at earlier releases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RestrictedName {
    Var,
    Yield,
    Record,
    Sealed,
    Permits,
}

impl RestrictedName {
    pub const ALL: [RestrictedName; 5] = [
        RestrictedName::Var,
        RestrictedName::Yield,
        RestrictedName::Record,
        RestrictedName::Sealed,
        RestrictedName::Permits,
    ];

    pub fn text(self) -> &'static str {
        match self {
            RestrictedName::Var => "var",
            RestrictedName::Yield => "yield",
            RestrictedName::Record => "record",
            RestrictedName::Sealed => "sealed",
            RestrictedName::Permits => "permits",
        }
    }

    pub fn from_text(text: &str) -> Option<RestrictedName> {
        RestrictedName::ALL.into_iter().find(|r| r.text() == text)
    }

    /// The first release where it can't name a type.
    pub fn restricted_from(self) -> u32 {
        match self {
            RestrictedName::Var => 10,
            RestrictedName::Yield => 14,
            RestrictedName::Record => 16,
            RestrictedName::Sealed | RestrictedName::Permits => 17,
        }
    }

    /// javac's code for the warning at earlier releases: `record`, `sealed`, and `permits` were
    /// restricted in preview releases first.
    pub fn javac_warning(self) -> &'static str {
        match self {
            RestrictedName::Var | RestrictedName::Yield => "restricted.type.not.allowed",
            RestrictedName::Record | RestrictedName::Sealed | RestrictedName::Permits => {
                "restricted.type.not.allowed.preview"
            }
        }
    }
}
