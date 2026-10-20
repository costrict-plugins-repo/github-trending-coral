use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::symbol::{CallReference, Symbol};

/// Supported source languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    #[cfg(feature = "lang-go")]
    Go,
    #[cfg(feature = "lang-java")]
    Java,
    #[cfg(feature = "lang-c")]
    C,
    #[cfg(feature = "lang-cpp")]
    Cpp,
}

/// A parsed source file with extracted symbols.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFile {
    pub path: std::path::PathBuf,
    pub language: Language,
    pub content_hash: String,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<String>,
    #[serde(default)]
    pub references: Vec<CallReference>,
}

/// Detect the language of a file from its extension.
pub fn detect_language(path: &Path) -> Option<Language> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "rs" => Some(Language::Rust),
        "ts" | "tsx" => Some(Language::TypeScript),
        "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
        "py" | "pyi" => Some(Language::Python),
        #[cfg(feature = "lang-go")]
        "go" => Some(Language::Go),
        #[cfg(feature = "lang-java")]
        "java" => Some(Language::Java),
        #[cfg(feature = "lang-c")]
        "c" => Some(Language::C),
        #[cfg(feature = "lang-c")]
        "h" => Some(Language::C),
        #[cfg(feature = "lang-cpp")]
        "cpp" | "cc" | "cxx" | "C" => Some(Language::Cpp),
        #[cfg(feature = "lang-cpp")]
        "hpp" | "hxx" | "hh" => Some(Language::Cpp),
        _ => None,
    }
}

/// Parse a single source file and extract symbols.
pub fn parse_file(path: &Path) -> Result<ParsedFile> {
    let language = detect_language(path)
        .with_context(|| format!("unsupported file type: {}", path.display()))?;

    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read file: {}", path.display()))?;

    parse_source(path, &source, language)
}

/// Parse source code from a string (useful for testing).
pub fn parse_source(path: &Path, source: &str, language: Language) -> Result<ParsedFile> {
    let content_hash = blake3::hash(source.as_bytes()).to_hex().to_string();

    let mut parser = tree_sitter::Parser::new();
    let ts_language = get_tree_sitter_language(language);
    parser
        .set_language(&ts_language)
        .context("failed to set tree-sitter language")?;

    let tree = parser
        .parse(source, None)
        .context("tree-sitter failed to parse source")?;

    let symbols = match language {
        Language::Rust => crate::extract_rust::extract_symbols(&tree, source),
        Language::TypeScript | Language::JavaScript => {
            crate::extract_ts::extract_symbols(&tree, source)
        }
        Language::Python => crate::extract_python::extract_symbols(&tree, source),
        #[cfg(feature = "lang-go")]
        Language::Go => crate::extract_go::extract_symbols(&tree, source),
        #[cfg(feature = "lang-java")]
        Language::Java => crate::extract_java::extract_symbols(&tree, source),
        #[cfg(feature = "lang-c")]
        Language::C => crate::extract_c::extract_symbols(&tree, source),
        #[cfg(feature = "lang-cpp")]
        Language::Cpp => crate::extract_cpp::extract_symbols(&tree, source),
    };

    let imports = extract_imports(&tree, source, language);
    let references = crate::references::extract_references(&tree, source, language);

    Ok(ParsedFile {
        path: path.to_path_buf(),
        language,
        content_hash,
        symbols,
        imports,
        references,
    })
}

/// Get the tree-sitter Language for a given Language enum.
fn get_tree_sitter_language(language: Language) -> tree_sitter::Language {
    match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::JavaScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        #[cfg(feature = "lang-go")]
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        #[cfg(feature = "lang-java")]
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        #[cfg(feature = "lang-c")]
        Language::C => tree_sitter_c::LANGUAGE.into(),
        #[cfg(feature = "lang-cpp")]
        Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
    }
}

/// Extract import statements from the AST.
fn extract_imports(tree: &tree_sitter::Tree, source: &str, language: Language) -> Vec<String> {
    let mut imports = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match language {
            Language::Rust => {
                if child.kind() == "use_declaration"
                    && let Some(text) = get_node_text(&child, source)
                {
                    imports.push(text.to_string());
                }
            }
            Language::TypeScript | Language::JavaScript => {
                if child.kind() == "import_statement"
                    && let Some(text) = get_node_text(&child, source)
                {
                    imports.push(text.to_string());
                }
            }
            Language::Python => {
                if (child.kind() == "import_statement"
                    || child.kind() == "import_from_statement")
                    && let Some(text) = get_node_text(&child, source)
                {
                    imports.push(text.to_string());
                }
            }
            #[cfg(feature = "lang-go")]
            Language::Go => {
                if child.kind() == "import_declaration"
                    && let Some(text) = get_node_text(&child, source)
                {
                    imports.push(text.to_string());
                }
            }
            #[cfg(feature = "lang-java")]
            Language::Java => {
                if child.kind() == "import_declaration"
                    && let Some(text) = get_node_text(&child, source)
                {
                    imports.push(text.to_string());
                }
            }
            #[cfg(feature = "lang-c")]
            Language::C => {
                if child.kind() == "preproc_include"
                    && let Some(text) = get_node_text(&child, source)
                {
                    imports.push(text.to_string());
                }
            }
            #[cfg(feature = "lang-cpp")]
            Language::Cpp => {
                if (child.kind() == "preproc_include" || child.kind() == "using_declaration")
                    && let Some(text) = get_node_text(&child, source)
                {
                    imports.push(text.to_string());
                }
            }
        }
    }

    imports
}

/// Get the text content of a tree-sitter node.
pub(crate) fn get_node_text<'a>(node: &tree_sitter::Node, source: &'a str) -> Option<&'a str> {
    node.utf8_text(source.as_bytes()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language_rust() {
        assert_eq!(
            detect_language(Path::new("src/main.rs")),
            Some(Language::Rust)
        );
    }

    #[test]
    fn test_detect_language_typescript() {
        assert_eq!(
            detect_language(Path::new("app/index.ts")),
            Some(Language::TypeScript)
        );
        assert_eq!(
            detect_language(Path::new("app/component.tsx")),
            Some(Language::TypeScript)
        );
    }

    #[test]
    fn test_detect_language_javascript() {
        assert_eq!(
            detect_language(Path::new("lib/utils.js")),
            Some(Language::JavaScript)
        );
        assert_eq!(
            detect_language(Path::new("lib/utils.mjs")),
            Some(Language::JavaScript)
        );
    }

    #[test]
    fn test_detect_language_python() {
        assert_eq!(
            detect_language(Path::new("app/main.py")),
            Some(Language::Python)
        );
        assert_eq!(
            detect_language(Path::new("stubs/module.pyi")),
            Some(Language::Python)
        );
    }

    #[cfg(feature = "lang-go")]
    #[test]
    fn test_detect_language_go() {
        assert_eq!(
            detect_language(Path::new("main.go")),
            Some(Language::Go)
        );
    }

    #[cfg(feature = "lang-java")]
    #[test]
    fn test_detect_language_java() {
        assert_eq!(
            detect_language(Path::new("App.java")),
            Some(Language::Java)
        );
    }

    #[cfg(feature = "lang-c")]
    #[test]
    fn test_detect_language_c() {
        assert_eq!(
            detect_language(Path::new("main.c")),
            Some(Language::C)
        );
        assert_eq!(
            detect_language(Path::new("header.h")),
            Some(Language::C)
        );
    }

    #[cfg(feature = "lang-cpp")]
    #[test]
    fn test_detect_language_cpp() {
        assert_eq!(
            detect_language(Path::new("main.cpp")),
            Some(Language::Cpp)
        );
        assert_eq!(
            detect_language(Path::new("main.cc")),
            Some(Language::Cpp)
        );
        assert_eq!(
            detect_language(Path::new("main.cxx")),
            Some(Language::Cpp)
        );
        assert_eq!(
            detect_language(Path::new("header.hpp")),
            Some(Language::Cpp)
        );
        assert_eq!(
            detect_language(Path::new("header.hxx")),
            Some(Language::Cpp)
        );
        assert_eq!(
            detect_language(Path::new("header.hh")),
            Some(Language::Cpp)
        );
    }

    #[test]
    fn test_detect_language_unknown() {
        assert_eq!(detect_language(Path::new("data.csv")), None);
        assert_eq!(detect_language(Path::new("Makefile")), None);
    }

    #[test]
    fn test_parse_source_rust() {
        let source = r#"
use std::io;

pub fn hello() -> String {
    "hello".to_string()
}
"#;
        let result =
            parse_source(Path::new("test.rs"), source, Language::Rust);
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.language, Language::Rust);
        assert!(!parsed.content_hash.is_empty());
        assert_eq!(parsed.imports.len(), 1);
        assert!(parsed.imports[0].contains("std::io"));
    }
}
