//! The config model, built from a config that passed validation, and resolution: which entries
//! apply to a path, and the settings they give it.

use std::collections::BTreeMap;
use std::fmt;

use ferrion_diagnostics::Span;
use globset::GlobSet;

use crate::glob::{self, Glob};
use crate::jsonc::{Member, Value, ValueKind};

/// A rule's level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Off,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn parse(s: &str) -> Option<Level> {
        match s {
            "off" => Some(Level::Off),
            "info" => Some(Level::Info),
            "warn" => Some(Level::Warn),
            "error" => Some(Level::Error),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A loaded, valid `ferrion.jsonc`. [`Config::default`] is the config used when there is no file:
/// it changes nothing.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub schema: Option<String>,
    pub plugins: Vec<String>,
    pub linter_options: LinterOptions,
    pub baseline: Option<String>,
    pub entries: Vec<Entry>,
    /// Every glob of every global ignore, with the entry and glob it came from.
    global_ignores: Option<(GlobSet, Vec<(usize, usize)>)>,
    /// The globs before `/**` of the global ignores that end in it, likewise.
    prunable: Option<(GlobSet, Vec<(usize, usize)>)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinterOptions {
    pub report_unused_disable_directives: Option<Level>,
}

/// An entry of `configs`.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The entry's index in `configs`.
    pub index: usize,
    pub name: Option<String>,
    /// The entry's object in the config.
    pub span: Span,
    pub files: Option<Vec<Glob>>,
    pub ignores: Option<Vec<Glob>>,
    /// The rule settings, in file order.
    pub rules: Option<Vec<Rule>>,
    pub format: Option<FormatSettings>,
    files_set: Option<GlobSet>,
    ignores_set: Option<GlobSet>,
}

impl Entry {
    /// Whether the entry has only `ignores` (and perhaps `name`): the files it matches are left
    /// out of every run.
    pub fn is_global_ignore(&self) -> bool {
        self.ignores.is_some()
            && self.files.is_none()
            && self.rules.is_none()
            && self.format.is_none()
    }

    /// `configs[2] "tests"`, or `configs[2]` without a name: how messages and `ferrion config`
    /// name the entry.
    pub fn label(&self) -> String {
        let mut label = format!("configs[{}]", self.index);
        if let Some(name) = &self.name {
            label.push(' ');
            crate::jsonc::write_json_string(&mut label, name);
        }
        label
    }

    /// Whether the entry applies to `path`, a normalized path relative to the working directory:
    /// when it has no `files` and the path ends in `.java`, or one of its `files` matches; and none
    /// of its `ignores` does. Global ignores are not treated apart here.
    pub fn matches(&self, path: &str) -> bool {
        let selected = match &self.files_set {
            Some(files) => files.is_match(path),
            None => path.ends_with(".java"),
        };
        selected && !self.ignores_set.as_ref().is_some_and(|i| i.is_match(path))
    }
}

/// A rule's setting in one entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    pub level: Level,
    /// The `options` object, if the setting has one.
    pub options: Option<Value>,
    /// The setting as written: a level string, or the object.
    pub value: Value,
}

/// An entry's `format` object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatSettings {
    pub enabled: bool,
}

impl Default for FormatSettings {
    fn default() -> FormatSettings {
        FormatSettings { enabled: true }
    }
}

/// A global ignore that matched: its entry's index, and the glob that matched.
#[derive(Clone, Copy, Debug)]
pub struct IgnoredBy<'c> {
    pub entry: &'c Entry,
    pub glob: &'c Glob,
}

impl fmt::Display for IgnoredBy<'_> {
    /// `ignored by configs[0], "**/legacy/**"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut glob = String::new();
        crate::jsonc::write_json_string(&mut glob, &self.glob.raw);
        write!(f, "ignored by {}, {glob}", self.entry.label())
    }
}

/// A setting and the entry it came from, or `None` for a default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sourced<T> {
    pub value: T,
    pub entry: Option<usize>,
}

/// The rule setting that applies to a file, and the entry it came from.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedRule<'c> {
    pub rule: &'c Rule,
    pub entry: usize,
}

/// What the config gives one file.
#[derive(Clone, Debug)]
pub struct Resolution<'c> {
    /// The global ignore that leaves the file out, if one does. The other fields are then empty
    /// and default.
    pub ignored: Option<IgnoredBy<'c>>,
    /// The indexes of the entries that match the file, in order. Global ignores aren't listed.
    pub entries: Vec<usize>,
    /// The `format` settings, from the last matching entry with a `format` object.
    pub format: Sourced<FormatSettings>,
    /// Each rule some matching entry mentions, by name, from the last entry that mentions it.
    pub rules: BTreeMap<&'c str, ResolvedRule<'c>>,
}

impl Config {
    /// The global ignore that leaves `path` out, if one does. `path` is normalized and relative to
    /// the working directory, with `/` separators. Of several, the first entry's first glob wins.
    pub fn ignored_by(&self, path: &str) -> Option<IgnoredBy<'_>> {
        self.first_match(self.global_ignores.as_ref()?, path)
    }

    /// Whether a folder can be skipped without reading it, because a global ignore ending in `/**`
    /// matches the folder and so everything under it. `folder` is normalized as a path is.
    pub fn prunes(&self, folder: &str) -> Option<IgnoredBy<'_>> {
        self.first_match(self.prunable.as_ref()?, folder)
    }

    fn first_match<'c>(
        &'c self,
        (set, origins): &(GlobSet, Vec<(usize, usize)>),
        path: &str,
    ) -> Option<IgnoredBy<'c>> {
        let first = set.matches(path).into_iter().min()?;
        let (entry, glob) = origins[first];
        let entry = &self.entries[entry];
        Some(IgnoredBy {
            entry,
            glob: &entry.ignores.as_ref().unwrap()[glob],
        })
    }

    /// What the config gives the file at `path`, normalized and relative to the working directory
    /// with `/` separators. The last matching entry that mentions a rule gives its whole setting,
    /// and the last matching entry with `format` gives all of it; nothing is merged.
    pub fn resolve(&self, path: &str) -> Resolution<'_> {
        if let Some(ignored) = self.ignored_by(path) {
            return Resolution {
                ignored: Some(ignored),
                entries: Vec::new(),
                format: Sourced {
                    value: FormatSettings::default(),
                    entry: None,
                },
                rules: BTreeMap::new(),
            };
        }
        self.resolve_entries(|entry| entry.matches(path))
    }

    /// What the config gives a file without a path, such as stdin without `--stdin-filepath`:
    /// only the entries without `files` and `ignores` apply.
    pub fn resolve_without_path(&self) -> Resolution<'_> {
        self.resolve_entries(|entry| entry.files.is_none() && entry.ignores.is_none())
    }

    fn resolve_entries(&self, applies: impl Fn(&Entry) -> bool) -> Resolution<'_> {
        let mut resolution = Resolution {
            ignored: None,
            entries: Vec::new(),
            format: Sourced {
                value: FormatSettings::default(),
                entry: None,
            },
            rules: BTreeMap::new(),
        };
        for entry in self
            .entries
            .iter()
            .filter(|e| !e.is_global_ignore() && applies(e))
        {
            resolution.entries.push(entry.index);
            if let Some(format) = entry.format {
                resolution.format = Sourced {
                    value: format,
                    entry: Some(entry.index),
                };
            }
            for rule in entry.rules.iter().flatten() {
                resolution.rules.insert(
                    &rule.name,
                    ResolvedRule {
                        rule,
                        entry: entry.index,
                    },
                );
            }
        }
        resolution
    }

    /// Builds the model from a root value that passed validation.
    pub(crate) fn build(root: &Value) -> Config {
        let mut config = Config::default();
        for member in root.as_object().unwrap() {
            let value = &member.value;
            match member.key.as_str() {
                "$schema" => config.schema = Some(string(value)),
                "plugins" => {
                    config.plugins = value.as_array().unwrap().iter().map(string).collect()
                }
                "baseline" => config.baseline = Some(string(value)),
                "linterOptions" => {
                    for option in value.as_object().unwrap() {
                        if option.key == "reportUnusedDisableDirectives" {
                            config.linter_options.report_unused_disable_directives =
                                Some(level(&option.value));
                        }
                    }
                }
                "configs" => {
                    config.entries = value
                        .as_array()
                        .unwrap()
                        .iter()
                        .enumerate()
                        .map(|(index, entry)| build_entry(index, entry))
                        .collect();
                }
                _ => unreachable!("validated key {}", member.key),
            }
        }
        let mut ignores = Vec::new();
        let mut prunable = Vec::new();
        for entry in config.entries.iter().filter(|e| e.is_global_ignore()) {
            for (i, glob) in entry.ignores.iter().flatten().enumerate() {
                ignores.push((glob.globset_glob().clone(), (entry.index, i)));
                if let Some(prefix) = glob.folder_prefix() {
                    prunable.push((prefix, (entry.index, i)));
                }
            }
        }
        config.global_ignores = indexed_set(ignores);
        config.prunable = indexed_set(prunable);
        config
    }
}

fn indexed_set(
    globs: Vec<(globset::Glob, (usize, usize))>,
) -> Option<(GlobSet, Vec<(usize, usize)>)> {
    if globs.is_empty() {
        return None;
    }
    let set = glob::set(globs.iter().map(|(g, _)| g));
    Some((set, globs.into_iter().map(|(_, origin)| origin).collect()))
}

fn build_entry(index: usize, value: &Value) -> Entry {
    let mut entry = Entry {
        index,
        name: None,
        span: value.span,
        files: None,
        ignores: None,
        rules: None,
        format: None,
        files_set: None,
        ignores_set: None,
    };
    for member in value.as_object().unwrap() {
        let value = &member.value;
        match member.key.as_str() {
            "name" => entry.name = Some(string(value)),
            "files" => entry.files = Some(globs(value)),
            "ignores" => entry.ignores = Some(globs(value)),
            "rules" => {
                entry.rules = Some(value.as_object().unwrap().iter().map(build_rule).collect())
            }
            "format" => {
                let mut format = FormatSettings::default();
                for setting in value.as_object().unwrap() {
                    if setting.key == "enabled" {
                        format.enabled = setting.value.kind == ValueKind::Bool(true);
                    }
                }
                entry.format = Some(format);
            }
            _ => unreachable!("validated key {}", member.key),
        }
    }
    entry.files_set = entry
        .files
        .as_ref()
        .map(|g| glob::set(g.iter().map(Glob::globset_glob)));
    entry.ignores_set = entry
        .ignores
        .as_ref()
        .map(|g| glob::set(g.iter().map(Glob::globset_glob)));
    entry
}

fn build_rule(member: &Member) -> Rule {
    let value = &member.value;
    let (level, options) = match &value.kind {
        ValueKind::String(_) => (self::level(value), None),
        ValueKind::Object(members) => {
            let level = members
                .iter()
                .find(|m| m.key == "level")
                .map(|m| self::level(&m.value))
                .unwrap();
            let options = members
                .iter()
                .find(|m| m.key == "options")
                .map(|m| m.value.clone());
            (level, options)
        }
        _ => unreachable!("validated rule setting"),
    };
    Rule {
        name: member.key.clone(),
        level,
        options,
        value: value.clone(),
    }
}

fn string(value: &Value) -> String {
    value.as_str().unwrap().to_owned()
}

fn level(value: &Value) -> Level {
    Level::parse(value.as_str().unwrap()).unwrap()
}

fn globs(value: &Value) -> Vec<Glob> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| Glob::new(item.as_str().unwrap(), item.span).unwrap())
        .collect()
}
