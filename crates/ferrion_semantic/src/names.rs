//! Interned names: every identifier, dotted name, and binary name the engine keeps is a [`Name`],
//! a 32-bit id given by the engine's [`Interner`] (the engine review, docs/design/semantic.md).
//!
//! A summary's names are most of its memory if each is its own string, and project symbols are
//! keyed and compared by name, so names are kept once and handled as ids: equal names have equal
//! ids, and comparing or hashing one is comparing or hashing a `u32`. Ids depend on the order
//! names were first seen, so they mean something only within one engine; anything compared
//! across engines (the differential check) is compared as text. The interner never forgets a
//! name, so a language server's interner grows with the distinct names it has seen, which is
//! small beside the summaries.

use std::collections::HashMap;
use std::fmt;
use std::hash::{BuildHasher, RandomState};
use std::sync::{Arc, RwLock};

use crate::engine::derived::{read, write};

/// A name interned by an [`Interner`]. Ordered by id, which is the order of first use, not by
/// text.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(u32);

impl Name {
    /// The empty name: a missing identifier, or the unnamed package. The same in every interner.
    pub const EMPTY: Name = Name(u32::MAX);

    pub fn is_empty(self) -> bool {
        self == Name::EMPTY
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Name({})", self.0)
    }
}

/// How many shards the interner has: enough that the summary phase's threads rarely want the same
/// shard's lock. A name's id is its index in its shard times this, plus the shard.
const SHARDS: usize = 64;

/// The names of one engine. Shared by every thread through `Arc`; interning takes one shard's
/// lock, and reading a name back takes it for reading.
pub struct Interner {
    shards: Box<[RwLock<Shard>]>,
    hasher: RandomState,
    paths: RwLock<Paths>,
}

/// A type's path of simple names (`Outer.Inner`), interned as its last name and the path of the
/// type it is a member of, so that a path is 4 bytes and making one is O(1) however deep the
/// type is nested. Ids mean something only within one engine, like names'.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Path(u32);

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Path({})", self.0)
    }
}

#[derive(Default)]
struct Paths {
    ids: HashMap<(Option<Path>, Name), Path>,
    paths: Vec<(Option<Path>, Name)>,
}

#[derive(Default)]
struct Shard {
    ids: HashMap<Arc<str>, Name>,
    names: Vec<Arc<str>>,
}

impl Default for Interner {
    fn default() -> Self {
        Interner::new()
    }
}

impl Interner {
    pub fn new() -> Interner {
        Interner {
            shards: (0..SHARDS).map(|_| RwLock::default()).collect(),
            hasher: RandomState::new(),
            paths: RwLock::default(),
        }
    }

    /// The path of the member type `simple` of the type at `outer`, or of the top-level type
    /// `simple` for `None`.
    pub fn path(&self, outer: Option<Path>, simple: Name) -> Path {
        let key = (outer, simple);
        if let Some(&path) = read(&self.paths).ids.get(&key) {
            return path;
        }
        let mut paths = write(&self.paths);
        if let Some(&path) = paths.ids.get(&key) {
            return path;
        }
        let path = Path(u32::try_from(paths.paths.len()).expect("fewer than four billion paths"));
        paths.paths.push(key);
        paths.ids.insert(key, path);
        path
    }

    /// The path of `names`, outermost first; `None` for no names.
    pub fn path_of(&self, names: &[Name]) -> Option<Path> {
        names
            .iter()
            .fold(None, |outer, &name| Some(self.path(outer, name)))
    }

    /// A path's simple names, outermost first. A loop up the path, however deep.
    pub fn path_names(&self, path: Path) -> Vec<Name> {
        let paths = read(&self.paths);
        let mut names = Vec::new();
        let mut next = Some(path);
        while let Some(p) = next {
            let (outer, name) = paths.paths[p.0 as usize];
            names.push(name);
            next = outer;
        }
        names.reverse();
        names
    }

    /// A path's last name: the type's simple name.
    pub fn path_last(&self, path: Path) -> Name {
        read(&self.paths).paths[path.0 as usize].1
    }

    /// The path joined by `separator`: `Outer.Inner`, or `Outer$Inner` for a binary name.
    pub fn path_text(&self, path: Path, separator: &str) -> String {
        let texts: Vec<Arc<str>> = self
            .path_names(path)
            .into_iter()
            .map(|n| self.text(n))
            .collect();
        texts.join(separator)
    }

    /// The id of `text`, interning it the first time.
    pub fn intern(&self, text: &str) -> Name {
        if text.is_empty() {
            return Name::EMPTY;
        }
        let s = self.shard_of(text);
        if let Some(&name) = read(&self.shards[s]).ids.get(text) {
            return name;
        }
        let mut shard = write(&self.shards[s]);
        if let Some(&name) = shard.ids.get(text) {
            return name;
        }
        let index = shard.names.len();
        let name = Name(
            u32::try_from(index * SHARDS + s).expect("fewer than four billion distinct names"),
        );
        let text: Arc<str> = Arc::from(text);
        shard.names.push(text.clone());
        shard.ids.insert(text, name);
        name
    }

    /// The id of `text` if it has been interned, without interning it: a name never seen can't
    /// be declared anywhere.
    pub fn lookup(&self, text: &str) -> Option<Name> {
        if text.is_empty() {
            return Some(Name::EMPTY);
        }
        read(&self.shards[self.shard_of(text)])
            .ids
            .get(text)
            .copied()
    }

    /// The text of `name`. Panics if `name` isn't one of this interner's.
    pub fn text(&self, name: Name) -> Arc<str> {
        if name.is_empty() {
            return Arc::from("");
        }
        let (s, index) = (name.0 as usize % SHARDS, name.0 as usize / SHARDS);
        read(&self.shards[s]).names[index].clone()
    }

    /// The number of distinct names, for measuring memory.
    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| read(s).names.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn shard_of(&self, text: &str) -> usize {
        self.hasher.hash_one(text) as usize % SHARDS
    }
}

impl fmt::Debug for Interner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Interner({} names)", self.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_text_equal_id() {
        let names = Interner::new();
        let a = names.intern("Map");
        let b = names.intern("Entry");
        assert_eq!(names.intern("Map"), a);
        assert_ne!(a, b);
        assert_eq!(&*names.text(a), "Map");
        assert_eq!(&*names.text(b), "Entry");
        assert_eq!(names.intern(""), Name::EMPTY);
        assert_eq!(&*names.text(Name::EMPTY), "");
        assert_eq!(names.lookup("Map"), Some(a));
        assert_eq!(names.lookup("List"), None);
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn many_names_across_threads() {
        let names = Interner::new();
        std::thread::scope(|scope| {
            for t in 0..4 {
                let names = &names;
                scope.spawn(move || {
                    for i in 0..1000 {
                        let text = format!("n{}", (i * 7 + t * 13) % 1000);
                        assert_eq!(&*names.text(names.intern(&text)), text);
                    }
                });
            }
        });
        assert_eq!(names.len(), 1000);
    }
}
