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
        }
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
