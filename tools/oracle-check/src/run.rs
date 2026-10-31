//! Running a check over the corpus: javac first, for what the cache doesn't have, then the
//! comparisons.
//!
//! javac runs in up to `jobs` JVMs at once, each analyzing a chunk of files at one release, so JVM
//! startup and warmup are paid once per chunk. Comparisons then run on `jobs` threads. Each file
//! is prepared once for all of its releases, and compared once for each distinct output javac gave
//! for it.

use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};

use crate::cache::{Cache, SourceKey};
use crate::check::{Check, Outcome, Status};
use crate::inventory::SourceFile;
use crate::oracle::Oracle;

/// Bounds on how much source text one JVM is given. Work is split into about three chunks per job,
/// within these bounds. Warming up a JVM costs more CPU time than scanning a few megabytes, so
/// chunks are large, but the smallest is enough to keep every job busy on a small run. With 10
/// jobs, the whole corpus at every release took 30 s, against 90 s with chunks of at most 4 MB.
const MAX_CHUNK_BYTES: usize = 64 << 20;
const MIN_CHUNK_BYTES: usize = 256 << 10;

pub struct FileResult {
    /// The index of the file in the inventory.
    pub file: usize,
    pub release: u32,
    pub outcome: Outcome,
}

#[derive(Default)]
pub struct Timings {
    pub javac_files: usize,
    pub javac_runs: usize,
    pub javac: Duration,
    pub compare: Duration,
}

pub fn run<C: Check>(
    check: &C,
    files: &[SourceFile],
    oracle: &Oracle,
    cache: &mut Cache,
    jobs: usize,
) -> Result<(Vec<FileResult>, Timings)> {
    let mut timings = Timings::default();
    let start = Instant::now();
    let failures = run_javac(files, oracle, cache, jobs, &mut timings)?;
    timings.javac = start.elapsed();

    let start = Instant::now();
    let results = compare(check, files, cache, &failures, jobs);
    timings.compare = start.elapsed();
    Ok((results, timings))
}

/// A chunk of work for one JVM: files at one release.
struct Chunk {
    release: u32,
    files: Vec<usize>,
}

/// Runs javac on every file and release the cache doesn't have. Returns the files javac produced
/// no output for, which aren't cached.
fn run_javac(
    files: &[SourceFile],
    oracle: &Oracle,
    cache: &mut Cache,
    jobs: usize,
    timings: &mut Timings,
) -> Result<HashMap<(u32, SourceKey), String>> {
    // Files with the same name and content need javac only once.
    let mut seen = HashSet::new();
    let mut needed: Vec<(u32, usize)> = Vec::new();
    for (i, file) in files.iter().enumerate() {
        if file.source.is_err() {
            continue;
        }
        for &release in &file.releases {
            if !cache.contains(release, &file.key) && seen.insert((release, file.key)) {
                needed.push((release, i));
            }
        }
    }
    needed.sort_by_key(|&(release, i)| (release, i));
    timings.javac_files = needed.len();
    if needed.is_empty() {
        return Ok(HashMap::new());
    }

    let total: usize = needed.iter().map(|&(_, i)| files[i].bytes.len()).sum();
    let target = (total / (jobs * 3)).clamp(MIN_CHUNK_BYTES, MAX_CHUNK_BYTES);
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut size = 0;
    for (release, i) in needed {
        match chunks.last_mut() {
            Some(chunk) if chunk.release == release && size < target => chunk.files.push(i),
            _ => {
                chunks.push(Chunk {
                    release,
                    files: vec![i],
                });
                size = 0;
            }
        }
        size += files[i].bytes.len();
    }
    timings.javac_runs = chunks.len();
    eprintln!(
        "running javac on {} files in {} JVMs, {} at a time",
        timings.javac_files,
        chunks.len(),
        jobs.min(chunks.len()),
    );

    // Workers take chunks from the end, so the last releases run first.
    let chunk_count = chunks.len();
    let chunks = Mutex::new(chunks);
    let (sender, receiver) = mpsc::channel::<Message>();
    let cache_ref: &Cache = cache;
    let received = std::thread::scope(|scope| -> Result<Vec<_>> {
        let mut workers = Vec::new();
        for worker in 0..jobs {
            let sender = sender.clone();
            let chunks = &chunks;
            workers.push(scope.spawn(move || -> Result<()> {
                loop {
                    let Some(chunk) = chunks.lock().unwrap().pop() else {
                        return Ok(());
                    };
                    let paths: Vec<&Path> = chunk
                        .files
                        .iter()
                        .map(|&i| files[i].path.as_path())
                        .collect();
                    let scratch = cache_ref.scratch(&format!("{}-{worker}", std::process::id()));
                    let mut stored = Ok(());
                    oracle.run(chunk.release, &paths, &scratch, |n, output| {
                        let key = files[chunk.files[n]].key;
                        let result = match output {
                            Ok(output) => match cache_ref.store(&output) {
                                Ok(hash) => Ok(hash),
                                Err(error) => {
                                    stored = Err(error);
                                    return;
                                }
                            },
                            Err(message) => Err(message),
                        };
                        sender
                            .send(Message::Output(chunk.release, key, result))
                            .ok();
                    })?;
                    stored?;
                    sender.send(Message::ChunkDone).ok();
                }
            }));
        }
        drop(sender);
        // Index lines are written here, on one thread, as outputs arrive, so an interrupted run
        // keeps what it finished.
        let progress = std::io::stderr().is_terminal();
        let mut done = 0;
        let mut received = Vec::new();
        let mut write_error = None;
        for message in receiver {
            match message {
                Message::Output(release, key, result) => {
                    if let Ok(hash) = &result
                        && let Err(error) = cache_ref.write_index(release, &key, hash)
                    {
                        write_error.get_or_insert(error);
                    }
                    received.push((release, key, result));
                }
                Message::ChunkDone => {
                    done += 1;
                    if progress {
                        eprint!("\r{done} of {chunk_count} JVMs done");
                    }
                }
            }
        }
        if progress {
            eprintln!();
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| anyhow!("a javac worker panicked"))??;
        }
        match write_error {
            Some(error) => Err(error),
            None => Ok(received),
        }
    })?;

    let mut failures = HashMap::new();
    for (release, key, result) in received {
        match result {
            Ok(hash) => cache.insert(release, key, hash),
            Err(message) => {
                failures.insert((release, key), message);
            }
        }
    }
    Ok(failures)
}

enum Message {
    /// A file's output hash at a release, or why there is none.
    Output(u32, SourceKey, Result<String, String>),
    ChunkDone,
}

fn compare<C: Check>(
    check: &C,
    files: &[SourceFile],
    cache: &Cache,
    failures: &HashMap<(u32, SourceKey), String>,
    jobs: usize,
) -> Vec<FileResult> {
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                let mut mine = Vec::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(i) else {
                        break;
                    };
                    let prepared = file
                        .source
                        .as_ref()
                        .map(|source| check.prepare(source, &file.bytes));
                    // javac's output is usually the same at every release, and then so is the
                    // outcome, so each output is compared once.
                    let mut compared: Vec<(&str, Outcome)> = Vec::new();
                    for &release in &file.releases {
                        let outcome = match (&prepared, cache.output_hash(release, &file.key)) {
                            (Err(reason), _) => {
                                Outcome::with_status(Status::Skipped((*reason).clone()))
                            }
                            (Ok(_), None) => Outcome::with_status(Status::JavacFailed(
                                failures
                                    .get(&(release, file.key))
                                    .cloned()
                                    .unwrap_or_else(|| "javac produced no output".to_owned()),
                            )),
                            (Ok(prepared), Some(hash)) => {
                                match compared.iter().find(|(seen, _)| *seen == hash) {
                                    Some((_, outcome)) => outcome.clone(),
                                    None => {
                                        let outcome = compare_output(check, prepared, cache, hash);
                                        compared.push((hash, outcome.clone()));
                                        outcome
                                    }
                                }
                            }
                        };
                        mine.push(FileResult {
                            file: i,
                            release,
                            outcome,
                        });
                    }
                }
                results.lock().unwrap().append(&mut mine);
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|result| (result.file, result.release));
    results
}

fn compare_output<C: Check>(
    check: &C,
    prepared: &C::Prepared,
    cache: &Cache,
    hash: &str,
) -> Outcome {
    let failed = |message: String| Outcome::with_status(Status::JavacFailed(message));
    match cache.read(hash) {
        Ok(output) => match output.failure() {
            Some(failure) => failed(failure.to_owned()),
            None => check.compare(prepared, &output),
        },
        Err(error) => failed(format!("{error:#}")),
    }
}
