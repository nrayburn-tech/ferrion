//! Checking a parsed config against the key tables in [`crate::keys`], collecting every error.

use crate::error::{ConfigError, ErrorCode};
use crate::glob::Glob;
use crate::jsonc::{Member, Value, ValueKind};
use crate::keys::{ENTRY_KEYS, Key, LEVELS, ROOT_KEYS, RULE_SETTING_KEYS, Shape};
use crate::suggest::did_you_mean;

/// Checks the root value, adding an error for everything wrong with it to `errors`.
pub(crate) fn check(root: &Value, errors: &mut Vec<ConfigError>) {
    let mut checker = Checker { errors };
    let Some(members) = root.as_object() else {
        checker.errors.push(
            ConfigError::new(
                ErrorCode::WrongType,
                "the config must be an object",
                root.span,
            )
            .label(format!("found {}", root.describe())),
        );
        return;
    };
    checker.object(members, root, ROOT_KEYS, &Place::root());
}

/// Where a value is, for messages.
#[derive(Clone, Debug)]
struct Place {
    /// The entry it is in: its index in `configs` and its name.
    entry: Option<(usize, Option<String>)>,
    /// The rule whose setting it is in.
    rule: Option<String>,
    /// Its path from the entry, or from the rule's setting, or from the root: `format.enabled`.
    path: String,
}

impl Place {
    fn root() -> Place {
        Place {
            entry: None,
            rule: None,
            path: String::new(),
        }
    }

    fn key(&self, key: &str) -> Place {
        let mut place = self.clone();
        if !place.path.is_empty() {
            place.path.push('.');
        }
        place.path.push_str(key);
        place
    }

    fn index(&self, i: usize) -> Place {
        let mut place = self.clone();
        place.path.push_str(&format!("[{i}]"));
        place
    }

    /// The value's name at the start of a message: "`format.enabled`", "`configs[1]`", "the
    /// level of rule `x`".
    fn subject(&self) -> String {
        if let Some(rule) = &self.rule {
            return match self.path.as_str() {
                "" => format!("the setting for rule `{rule}`"),
                "level" => format!("the level of rule `{rule}`"),
                "options" => format!("the options of rule `{rule}`"),
                path => format!("`{path}` of rule `{rule}`"),
            };
        }
        match (&self.entry, self.path.is_empty()) {
            (Some((index, _)), true) => format!("`configs[{index}]`"),
            _ => format!("`{}`", self.path),
        }
    }

    /// The object's name after "in", for an unknown key in it: "the config", "configs[1]
    /// \"tests\"", "configs[1].format".
    fn location(&self) -> String {
        if let Some(rule) = &self.rule {
            return match self.path.as_str() {
                "" => format!("the setting for rule `{rule}`"),
                path => format!("`{path}` of rule `{rule}`"),
            };
        }
        match &self.entry {
            None if self.path.is_empty() => "the config".to_owned(),
            None => self.path.clone(),
            Some((index, name)) => {
                let mut location = format!("configs[{index}]");
                if self.path.is_empty() {
                    if let Some(name) = name {
                        location.push(' ');
                        crate::jsonc::write_json_string(&mut location, name);
                    }
                } else {
                    location.push('.');
                    location.push_str(&self.path);
                }
                location
            }
        }
    }
}

struct Checker<'e> {
    errors: &'e mut Vec<ConfigError>,
}

impl Checker<'_> {
    fn wrong_type(&mut self, value: &Value, place: &Place, expected: &str) -> ConfigError {
        ConfigError::new(
            ErrorCode::WrongType,
            format!("{} must be {expected}", place.subject()),
            value.span,
        )
        .label(format!("found {}", value.describe()))
    }

    fn push(&mut self, error: ConfigError) {
        self.errors.push(error);
    }

    /// Checks an object's members against `keys`, and that it has every required key.
    fn object(&mut self, members: &[Member], value: &Value, keys: &[Key], place: &Place) {
        for member in members {
            match keys.iter().find(|k| k.name == member.key) {
                Some(key) => self.shape(&member.value, key.shape, &place.key(key.name)),
                None => self.unknown_key(member, keys, place),
            }
        }
        for key in keys.iter().filter(|k| k.required) {
            if !members.iter().any(|m| m.key == key.name) {
                self.push(
                    ConfigError::new(
                        ErrorCode::MissingKey,
                        format!("{} has no `{}`", place.subject(), key.name),
                        value.span,
                    )
                    .help(match key.shape {
                        Shape::Level => format!(
                            "add `\"{}\": \"error\"`, or another level: {}",
                            key.name,
                            levels_list("or")
                        ),
                        _ => format!("add `\"{}\"`", key.name),
                    }),
                );
            }
        }
    }

    fn unknown_key(&mut self, member: &Member, keys: &[Key], place: &Place) {
        let names = keys.iter().map(|k| k.name);
        let suggestion = did_you_mean(&member.key, names.clone());
        let mut error = ConfigError::new(
            ErrorCode::UnknownKey,
            format!("unknown key `{}` in {}", member.key, place.location()),
            member.key_span,
        )
        .suggest(suggestion);
        if suggestion.is_none() {
            let elsewhere = if same_keys(keys, ROOT_KEYS) {
                ENTRY_KEYS
                    .iter()
                    .any(|k| k.name == member.key)
                    .then(|| format!("`{}` goes in an entry of `configs`", member.key))
            } else if same_keys(keys, ENTRY_KEYS) {
                ROOT_KEYS.iter().any(|k| k.name == member.key).then(|| {
                    format!(
                        "`{}` goes in the config's root object, not in an entry",
                        member.key
                    )
                })
            } else {
                None
            };
            error = error.help(elsewhere.unwrap_or_else(|| {
                let names: Vec<String> = keys.iter().map(|k| format!("`{}`", k.name)).collect();
                match names.len() {
                    1 => format!("the only key here is {}", names[0]),
                    _ => format!("the keys here are {}", list(&names, "and")),
                }
            }));
        }
        self.push(error);
    }

    fn shape(&mut self, value: &Value, shape: Shape, place: &Place) {
        match shape {
            Shape::String => {
                if value.as_str().is_none() {
                    let error = self.wrong_type(value, place, "a string");
                    self.push(error);
                }
            }
            Shape::Bool => {
                if !matches!(value.kind, ValueKind::Bool(_)) {
                    let error = self.wrong_type(value, place, "true or false");
                    self.push(error);
                }
            }
            Shape::Level => self.level(value, place),
            Shape::Globs { non_empty } => self.globs(value, place, non_empty),
            Shape::Names => self.names(value, place),
            Shape::Object(keys) => match value.as_object() {
                Some(members) => self.object(members, value, keys, place),
                None => {
                    let error = self.wrong_type(value, place, "an object");
                    self.push(error);
                }
            },
            Shape::Entries => self.entries(value, place),
            Shape::Rules => self.rules(value, place),
            Shape::RuleSetting => match &value.kind {
                ValueKind::String(_) => self.level(value, place),
                ValueKind::Object(members) => {
                    self.object(members, value, RULE_SETTING_KEYS, place);
                }
                _ => {
                    let error = self
                        .wrong_type(value, place, "a level or an object")
                        .help(format!(
                            "a rule's setting is a level ({}), or `{{ \"level\": ..., \"options\": {{ ... }} }}`",
                            levels_list("or")
                        ));
                    self.push(error);
                }
            },
            Shape::Options => {
                if value.as_object().is_none() {
                    let error = self.wrong_type(value, place, "an object");
                    self.push(error);
                }
            }
        }
    }

    fn level(&mut self, value: &Value, place: &Place) {
        match value.as_str() {
            Some(level) if LEVELS.contains(&level) => {}
            Some(level) => {
                let suggestion = did_you_mean(level, LEVELS);
                let mut error = ConfigError::new(
                    ErrorCode::UnknownLevel,
                    format!("unknown level `{level}`"),
                    value.span,
                )
                .suggest(suggestion);
                if suggestion.is_none() {
                    error = error.help(format!("the levels are {}", levels_list("and")));
                }
                self.push(error);
            }
            None => {
                let expected = if place.path == "level" {
                    "a string"
                } else {
                    "a level"
                };
                let error = self
                    .wrong_type(value, place, expected)
                    .help(format!("the levels are {}", levels_list("and")));
                self.push(error);
            }
        }
    }

    fn globs(&mut self, value: &Value, place: &Place, non_empty: bool) {
        let Some(items) = value.as_array() else {
            let error = self.wrong_type(value, place, "an array of globs");
            self.push(error);
            return;
        };
        if non_empty && items.is_empty() {
            self.push(
                ConfigError::new(
                    ErrorCode::EmptyFiles,
                    format!("{} can't be empty", place.subject()),
                    value.span,
                )
                .label("this entry would apply to no file")
                .help("to apply the entry to every `*.java` file, leave `files` out"),
            );
        }
        for (i, item) in items.iter().enumerate() {
            let Some(raw) = item.as_str() else {
                let error = self.wrong_type(item, &place.index(i), "a string");
                self.push(error);
                continue;
            };
            if let Err(e) = Glob::new(raw, item.span) {
                let mut error = ConfigError::new(ErrorCode::InvalidGlob, e.message, item.span);
                error.label = e.label;
                error.help = e.help;
                error.suggestion = e.suggestion;
                self.push(error);
            }
        }
    }

    fn names(&mut self, value: &Value, place: &Place) {
        let Some(items) = value.as_array() else {
            let error = self.wrong_type(value, place, "an array of names");
            self.push(error);
            return;
        };
        for (i, item) in items.iter().enumerate() {
            if item.as_str().is_none() {
                let error = self.wrong_type(item, &place.index(i), "a string");
                self.push(error);
            }
        }
    }

    fn entries(&mut self, value: &Value, place: &Place) {
        let Some(items) = value.as_array() else {
            let error = self.wrong_type(value, place, "an array of entries");
            self.push(error);
            return;
        };
        for (i, item) in items.iter().enumerate() {
            let Some(members) = item.as_object() else {
                let error = self
                    .wrong_type(item, &place.index(i), "an object")
                    .help("an entry is an object with `files`, `ignores`, `rules`, or `format`");
                self.push(error);
                continue;
            };
            let name = members
                .iter()
                .find(|m| m.key == "name")
                .and_then(|m| m.value.as_str())
                .map(str::to_owned);
            let entry = Place {
                entry: Some((i, name)),
                rule: None,
                path: String::new(),
            };
            self.object(members, item, ENTRY_KEYS, &entry);
        }
    }

    fn rules(&mut self, value: &Value, place: &Place) {
        let Some(members) = value.as_object() else {
            let error = self.wrong_type(value, place, "an object");
            self.push(error);
            return;
        };
        for member in members {
            let rule = Place {
                entry: place.entry.clone(),
                rule: Some(member.key.clone()),
                path: String::new(),
            };
            self.shape(&member.value, Shape::RuleSetting, &rule);
        }
    }
}

fn same_keys(a: &[Key], b: &[Key]) -> bool {
    a.iter().map(|k| k.name).eq(b.iter().map(|k| k.name))
}

/// "`off`, `info`, `warn`, and `error`".
fn levels_list(conjunction: &str) -> String {
    let names: Vec<String> = LEVELS.iter().map(|l| format!("`{l}`")).collect();
    list(&names, conjunction)
}

/// "a, b, and c", or with another conjunction.
fn list(items: &[String], conjunction: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} {conjunction} {b}"),
        [init @ .., last] => format!("{}, {conjunction} {last}", init.join(", ")),
    }
}
