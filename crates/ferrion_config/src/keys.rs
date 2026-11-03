//! The keys of `ferrion.jsonc`'s objects and the shapes of their values. The validator checks a
//! config against these tables, and `cargo codegen` generates the JSON Schema from them, so the two
//! can't disagree.

/// A key of an object, and what its value must be.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    pub name: &'static str,
    pub shape: Shape,
    /// Whether the object must have this key.
    pub required: bool,
    /// What the key means, for the schema's `description`.
    pub description: &'static str,
}

/// What a value must be.
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    /// Any string.
    String,
    Bool,
    /// One of [`LEVELS`].
    Level,
    /// An array of globs; with `non_empty`, at least one.
    Globs {
        non_empty: bool,
    },
    /// An array of names (strings). The names themselves are checked from C1.
    Names,
    /// An object with exactly these keys, each optional unless it says otherwise.
    Object(&'static [Key]),
    /// An array of entries, each an [`Shape::Object`] of [`ENTRY_KEYS`].
    Entries,
    /// An object of rule names to [`Shape::RuleSetting`]s. The names are checked from C1.
    Rules,
    /// A level, or an object of [`RULE_SETTING_KEYS`].
    RuleSetting,
    /// Any object: a rule's options, which are checked from C1.
    Options,
}

/// The levels of a rule, in increasing severity.
pub const LEVELS: [&str; 4] = ["off", "info", "warn", "error"];

pub const ROOT_KEYS: &[Key] = &[
    Key {
        name: "$schema",
        shape: Shape::String,
        required: false,
        description: "The JSON Schema of this file, for editors.",
    },
    Key {
        name: "configs",
        shape: Shape::Entries,
        required: false,
        description: "The config entries, in order: for each file, later entries that match it win.",
    },
    Key {
        name: "plugins",
        shape: Shape::Names,
        required: false,
        description: "The rule plugins to load, by name.",
    },
    Key {
        name: "linterOptions",
        shape: Shape::Object(LINTER_OPTIONS_KEYS),
        required: false,
        description: "Options of the linter as a whole.",
    },
    Key {
        name: "baseline",
        shape: Shape::String,
        required: false,
        description: "The baseline file, relative to the working directory.",
    },
];

pub const ENTRY_KEYS: &[Key] = &[
    Key {
        name: "name",
        shape: Shape::String,
        required: false,
        description: "A name for the entry, shown in `ferrion config` and in errors.",
    },
    Key {
        name: "files",
        shape: Shape::Globs { non_empty: true },
        required: false,
        description: "Globs of the files the entry applies to, relative to the working directory. \
                      Without it, the entry applies to every `*.java` file.",
    },
    Key {
        name: "ignores",
        shape: Shape::Globs { non_empty: false },
        required: false,
        description: "Globs of files the entry doesn't apply to. In an entry with only `ignores` \
                      (and `name`), the files are left out of every run.",
    },
    Key {
        name: "rules",
        shape: Shape::Rules,
        required: false,
        description: "Rule settings: a level, or an object with `level` and `options`. The last \
                      matching entry that mentions a rule gives its whole setting.",
    },
    Key {
        name: "format",
        shape: Shape::Object(FORMAT_KEYS),
        required: false,
        description: "Formatter settings. The last matching entry with `format` gives all of them.",
    },
];

pub const LINTER_OPTIONS_KEYS: &[Key] = &[Key {
    name: "reportUnusedDisableDirectives",
    shape: Shape::Level,
    required: false,
    description: "The level at which to report a disable comment that disables nothing.",
}];

pub const FORMAT_KEYS: &[Key] = &[Key {
    name: "enabled",
    shape: Shape::Bool,
    required: false,
    description: "Whether `ferrion format` formats the files. It does unless this is false.",
}];

pub const RULE_SETTING_KEYS: &[Key] = &[
    Key {
        name: "level",
        shape: Shape::Level,
        required: true,
        description: "The rule's level.",
    },
    Key {
        name: "options",
        shape: Shape::Options,
        required: false,
        description: "The rule's options.",
    },
];
