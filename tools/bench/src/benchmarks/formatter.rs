use std::hint::black_box;

use ferrion_diagnostics::SourceText;
use ferrion_formatter::format_tree;
use ferrion_parser::{ParseOptions, Severity, parse};

use super::{Benchmark, Inputs, Prepared, corpus_sources};

pub fn benchmarks() -> Vec<Benchmark> {
    vec![Benchmark {
        name: "formatter/corpus",
        description: "format every source file of the corpus projects that parses without errors \
                      at its source set's release, from trees parsed in setup",
        setup: corpus,
    }]
}

fn corpus(inputs: &Inputs) -> anyhow::Result<Prepared> {
    let (input, files) = corpus_sources(inputs)?;
    // The trees are parsed here, outside the measurement, so an iteration is `format_tree`
    // alone. Files with errors are skipped, as `format` skips them.
    let mut trees = Vec::new();
    for file in files {
        let parsed = parse(
            file.text.clone(),
            ParseOptions {
                release: file.release,
            },
        );
        if parsed
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
        {
            continue;
        }
        // The line ending and byte-order mark come from the text: corpus_sources has decoded the
        // file already, so a byte-order mark is gone, and formatting adds none.
        let source = SourceText::decode(file.text.as_bytes())?;
        trees.push((parsed.tree, source));
    }
    let bytes = trees.iter().map(|(tree, _)| tree.text().len() as u64).sum();
    let input = format!("{input}, {} formatted", trees.len());
    Ok(Prepared {
        input,
        bytes: Some(bytes),
        run: Box::new(move || {
            for (tree, source) in &trees {
                black_box(format_tree(tree, source));
            }
        }),
    })
}
