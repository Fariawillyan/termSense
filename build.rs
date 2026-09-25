//! Builds the search index of the built-in knowledge at compile time and
//! embeds it in the binary: `ts` then decodes it at startup instead of
//! indexing ~200 KB of text on every run. The same modules as the program
//! are compiled here, so the index is identical to one built at runtime
//! (a test checks that).

#![allow(dead_code)]

use std::path::PathBuf;
use std::{env, fs};

#[path = "src/knowledge"]
mod knowledge {
    #[path = "loader.rs"]
    pub mod loader;
    #[path = "model.rs"]
    pub mod model;
    #[path = "template.rs"]
    pub mod template;
}

#[path = "src/search"]
mod search {
    #[path = "index.rs"]
    pub mod index;
    #[path = "tokenizer.rs"]
    pub mod tokenizer;
}

fn main() {
    println!("cargo:rerun-if-changed=knowledge");
    for f in [
        "src/knowledge/loader.rs",
        "src/knowledge/model.rs",
        "src/knowledge/template.rs",
        "src/search/index.rs",
        "src/search/tokenizer.rs",
    ] {
        println!("cargo:rerun-if-changed={f}");
    }
    let loaded = knowledge::loader::load_embedded()
        .unwrap_or_else(|e| panic!("base de conhecimento embutida inválida: {e}"));
    let index = search::index::build(&loaded.entries);
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("index.bin");
    fs::write(&out, index.encode()).expect("gravar o índice");
}
