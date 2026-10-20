//! # coral-index
//!
//! Tree-sitter AST parsing and symbol extraction for Rust, TypeScript, Python, Go, Java, C, and C++.
//!
//! This crate provides:
//! - Multi-language source file parsing via tree-sitter
//! - Symbol extraction (functions, structs, classes, interfaces, etc.)
//! - Full repository indexing with rayon parallelism
//! - Incremental re-indexing based on blake3 content hashes
//! - Serialization of the index to/from disk

pub(crate) mod extract_helpers;
#[cfg(feature = "lang-go")]
pub mod extract_go;
#[cfg(feature = "lang-java")]
pub mod extract_java;
#[cfg(feature = "lang-c")]
pub mod extract_c;
#[cfg(feature = "lang-cpp")]
pub mod extract_cpp;
pub mod extract_python;
pub mod extract_rust;
pub mod extract_ts;
pub mod indexer;
pub mod parser;
pub mod references;
pub mod store;
pub mod symbol;

// Re-export key types for convenience.
pub use indexer::{IndexConfig, RepoIndex, index_incremental, index_repo};
pub use parser::{Language, ParsedFile, detect_language, parse_file, parse_source};
pub use store::{load_index, save_index};
pub use symbol::{CallReference, Span, Symbol, SymbolKind, Visibility};
