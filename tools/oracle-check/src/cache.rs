//! The cache of the oracle's output, so reruns don't run javac again.
//!
//! ```text
//! target/oracle/<oracle key>/          one directory per oracle jar, JDK, and set of sections
//!   outputs/<ab>/<sha256>.json         one file's output, named by its own hash
//!   index/<release>.txt                lines of `<source key> <output hash>`
//! ```
//!
//! A source key hashes a file's name and content, which is all the output depends on besides the
//! oracle key and the release. Outputs are stored by their own hash, so a file whose output is the
//! same at every release, as most are, is stored once. Index lines are appended as outputs arrive,
//! so an interrupted run keeps what it finished. A different jar or JDK gets a new directory, and
//! old ones can be deleted.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::oracle::{Output, hex};

pub type SourceKey = [u8; 32];

pub fn source_key(file_name: &str, bytes: &[u8]) -> SourceKey {
    let mut hash = Sha256::new();
    hash.update(file_name);
    hash.update(b"\0");
    hash.update(bytes);
    hash.finalize().into()
}

pub struct Cache {
    dir: PathBuf,
    /// For each release and source key, the hash of the output.
    index: HashMap<(u32, SourceKey), String>,
}

impl Cache {
    /// Opens the cache for `oracle_key` under `root`, loading the index for `releases`.
    pub fn open(root: &Path, oracle_key: &str, releases: &[u32]) -> Result<Cache> {
        let dir = root.join(oracle_key);
        std::fs::create_dir_all(dir.join("index"))?;
        std::fs::create_dir_all(dir.join("outputs"))?;
        std::fs::create_dir_all(dir.join("scratch"))?;
        let mut index = HashMap::new();
        for &release in releases {
            let path = index_file(&dir, release);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if !text.is_empty() && !text.ends_with('\n') {
                // Ends the line an interrupted run cut short, so the next line starts afresh.
                OpenOptions::new()
                    .append(true)
                    .open(&path)?
                    .write_all(b"\n")?;
            }
            for line in text.lines() {
                // A line cut short by an interrupted run is ignored.
                let Some((source, output)) = line.split_once(' ') else {
                    continue;
                };
                let (Some(source), true) = (parse_key(source), is_hash(output)) else {
                    continue;
                };
                index.insert((release, source), output.to_owned());
            }
        }
        Ok(Cache { dir, index })
    }

    /// Where a JVM's list file and error output go.
    pub fn scratch(&self, name: &str) -> PathBuf {
        self.dir.join("scratch").join(name)
    }

    /// The hash of a file's output at a release, if it's cached.
    pub fn output_hash(&self, release: u32, source: &SourceKey) -> Option<&str> {
        self.index.get(&(release, *source)).map(String::as_str)
    }

    pub fn read(&self, hash: &str) -> Result<Output> {
        let path = self.output_file(hash);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("parsing {}; delete it to rerun javac", path.display()))
    }

    pub fn contains(&self, release: u32, source: &SourceKey) -> bool {
        self.index
            .get(&(release, *source))
            .is_some_and(|hash| self.output_file(hash).exists())
    }

    /// Stores an output, returning its hash for [`Cache::write_index`]. Safe to call from several
    /// threads.
    pub fn store(&self, output: &Output) -> Result<String> {
        let text = serde_json::to_string(output)?;
        let hash = hex(&Sha256::digest(text.as_bytes()));
        let path = self.output_file(&hash);
        if !path.exists() {
            std::fs::create_dir_all(path.parent().unwrap())?;
            // Written under a temporary name, so a reader never sees half a file.
            let partial = path.with_extension(format!(
                "partial-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::write(&partial, text)?;
            std::fs::rename(&partial, &path)?;
        }
        Ok(hash)
    }

    /// Records that a file's output at a release is the stored output `hash`, for later runs.
    /// Safe to call from several threads.
    pub fn write_index(&self, release: u32, source: &SourceKey, hash: &str) -> Result<()> {
        let path = index_file(&self.dir, release);
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        file.write_all(format!("{} {hash}\n", hex(source)).as_bytes())?;
        Ok(())
    }

    /// Makes a recorded output available to this run.
    pub fn insert(&mut self, release: u32, source: SourceKey, hash: String) {
        self.index.insert((release, source), hash);
    }

    fn output_file(&self, hash: &str) -> PathBuf {
        self.dir
            .join("outputs")
            .join(&hash[..2])
            .join(format!("{hash}.json"))
    }
}

fn index_file(dir: &Path, release: u32) -> PathBuf {
    dir.join("index").join(format!("{release}.txt"))
}

fn is_hash(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse_key(text: &str) -> Option<SourceKey> {
    if !is_hash(text) {
        return None;
    }
    let mut key = [0; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oracle-check-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    fn record(cache: &mut Cache, release: u32, source: SourceKey, hash: String) {
        cache.write_index(release, &source, &hash).unwrap();
        cache.insert(release, source, hash);
    }

    #[test]
    fn outputs_survive_reopening_and_are_shared_between_releases() {
        let root = temp_dir("cache");
        let output = Output {
            tokens: Some(vec![("EOF".into(), 0, 0)]),
            ..Output::default()
        };
        let a = source_key("A.java", b"");
        let b = source_key("B.java", b"");
        assert_ne!(a, b);

        let mut cache = Cache::open(&root, "key", &[8, 17]).unwrap();
        assert!(!cache.contains(8, &a));
        let hash = cache.store(&output).unwrap();
        assert_eq!(cache.store(&output).unwrap(), hash);
        record(&mut cache, 8, a, hash.clone());
        record(&mut cache, 17, a, hash);
        assert_eq!(
            cache.read(cache.output_hash(8, &a).unwrap()).unwrap(),
            output
        );

        // A torn line from an interrupted run is ignored.
        let index = root.join("key/index/17.txt");
        let mut file = OpenOptions::new().append(true).open(&index).unwrap();
        file.write_all(b"0123").unwrap();

        let mut cache = Cache::open(&root, "key", &[8, 17]).unwrap();
        assert!(cache.contains(8, &a) && cache.contains(17, &a));
        assert!(!cache.contains(8, &b));
        assert_eq!(cache.output_hash(17, &a), cache.output_hash(8, &a));
        assert_eq!(
            cache.read(cache.output_hash(17, &a).unwrap()).unwrap(),
            output
        );
        assert_eq!(cache.output_hash(8, &b), None);
        let hash = cache.store(&output).unwrap();
        record(&mut cache, 17, b, hash);
        let cache = Cache::open(&root, "key", &[17]).unwrap();
        assert!(cache.contains(17, &b));
        let stored = std::fs::read_dir(root.join("key/outputs")).unwrap().count();
        assert_eq!(stored, 1);
        std::fs::remove_dir_all(&root).ok();
    }
}
