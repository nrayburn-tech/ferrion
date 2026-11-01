use std::hint::black_box;

use super::{Benchmark, Inputs, Prepared, corpus_sources};

pub fn benchmarks() -> Vec<Benchmark> {
    vec![Benchmark {
        name: "lexer/corpus",
        description: "lex every source file of the corpus projects",
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
                black_box(ferrion_lexer::lex(&file.text));
            }
        }),
    })
}
