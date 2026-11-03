//! The `ferrion` command-line interface.

// File selection is built ahead of the commands that use it.
#[allow(dead_code)]
mod files;

fn main() {
    println!("ferrion {}", env!("CARGO_PKG_VERSION"));
}
