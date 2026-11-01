use std::hint::black_box;

use ferrion_parser::{ParseOptions, parse};

use super::{Benchmark, Inputs, Prepared, corpus_sources};

pub fn benchmarks() -> Vec<Benchmark> {
    vec![Benchmark {
        name: "parser/corpus",
        description: "lex and parse every source file of the corpus projects at its source set's \
                      release, dropping each tree before the next",
        setup: corpus,
    }]
}

fn corpus(inputs: &Inputs) -> anyhow::Result<Prepared> {
    let (input, files) = corpus_sources(inputs)?;
    let bytes = files.iter().map(|file| file.text.len() as u64).sum();
    Ok(Prepared {
        input,
        bytes: Some(bytes),
        run: Box::new(move || {
            for file in &files {
                let options = ParseOptions {
                    release: file.release,
                };
                black_box(parse(file.text.clone(), options));
            }
        }),
    })
}
