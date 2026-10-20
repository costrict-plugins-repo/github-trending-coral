use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{
    Field, NumericOptions, Schema, TextFieldIndexing, TextOptions, Value, STORED, STRING, TEXT,
};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument};

use crate::types::{DocType, SearchResult, SearchSource};

/// A document to be indexed by the keyword search engine.
#[derive(Debug, Clone)]
pub struct IndexDocument {
    pub path: PathBuf,
    pub content: String,
    pub symbol: Option<String>,
    pub language: Option<String>,
    pub doc_type: DocType,
    pub line: Option<u32>,
}

/// Field handles for the tantivy schema.
struct SchemaFields {
    path: Field,
    content: Field,
    symbol: Field,
    language: Field,
    doc_type: Field,
    line: Field,
}

/// Tantivy-based BM25 keyword search index.
pub struct KeywordIndex {
    index: Index,
    reader: IndexReader,
    fields: SchemaFields,
}

fn build_schema() -> (Schema, SchemaFields) {
    let mut schema_builder = Schema::builder();

    let path = schema_builder.add_text_field("path", STRING | STORED);

    let text_indexing = TextFieldIndexing::default()
        .set_tokenizer("default")
        .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions);
    let text_options = TextOptions::default()
        .set_indexing_options(text_indexing)
        .set_stored();
    let content = schema_builder.add_text_field("content", text_options);

    let symbol = schema_builder.add_text_field("symbol", TEXT | STORED);
    let language = schema_builder.add_text_field("language", STRING | STORED);
    let doc_type = schema_builder.add_text_field("doc_type", STRING | STORED);
    let line = schema_builder.add_u64_field("line", NumericOptions::default().set_stored());

    let schema = schema_builder.build();
    let fields = SchemaFields {
        path,
        content,
        symbol,
        language,
        doc_type,
        line,
    };
    (schema, fields)
}

impl KeywordIndex {
    /// Create a new index at the given filesystem path.
    pub fn create(index_path: &Path) -> Result<Self> {
        std::fs::create_dir_all(index_path)
            .with_context(|| format!("failed to create index directory: {}", index_path.display()))?;

        let (schema, fields) = build_schema();
        let index = Index::create_in_dir(index_path, schema)
            .context("failed to create tantivy index")?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("failed to create index reader")?;

        Ok(Self {
            index,
            reader,
            fields,
        })
    }

    /// Open an existing index from the given path.
    pub fn open(index_path: &Path) -> Result<Self> {
        let (_schema, fields) = build_schema();
        let index =
            Index::open_in_dir(index_path).context("failed to open tantivy index")?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("failed to create index reader")?;

        Ok(Self {
            index,
            reader,
            fields,
        })
    }

    /// Create an in-RAM index (useful for tests).
    pub fn create_in_ram() -> Result<Self> {
        let (schema, fields) = build_schema();
        let index = Index::create_in_ram(schema);

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("failed to create index reader")?;

        Ok(Self {
            index,
            reader,
            fields,
        })
    }

    /// Index a single document.
    pub fn add_document(&self, doc: &IndexDocument) -> Result<()> {
        let mut writer: IndexWriter = self
            .index
            .writer(50_000_000)
            .context("failed to create index writer")?;

        self.write_document(&mut writer, doc)?;
        writer.commit().context("failed to commit index")?;
        self.reader.reload().context("failed to reload reader")?;

        Ok(())
    }

    /// Bulk index multiple documents in a single commit.
    pub fn add_documents(&self, docs: &[IndexDocument]) -> Result<()> {
        let mut writer: IndexWriter = self
            .index
            .writer(50_000_000)
            .context("failed to create index writer")?;

        for doc in docs {
            self.write_document(&mut writer, doc)?;
        }
        writer.commit().context("failed to commit index")?;
        self.reader.reload().context("failed to reload reader")?;

        Ok(())
    }

    /// Search with BM25 scoring, returning up to `limit` results.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.fields.content, self.fields.symbol]);

        let parsed_query = query_parser
            .parse_query(query)
            .with_context(|| format!("failed to parse query: {query}"))?;

        let top_docs = searcher
            .search(&parsed_query, &TopDocs::with_limit(limit))
            .context("search failed")?;

        let mut results = Vec::with_capacity(top_docs.len());

        for (score, doc_address) in top_docs {
            let retrieved: TantivyDocument = searcher
                .doc(doc_address)
                .context("failed to retrieve document")?;

            let path = retrieved
                .get_first(self.fields.path)
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            let snippet = retrieved
                .get_first(self.fields.content)
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            let symbol = retrieved
                .get_first(self.fields.symbol)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());

            let line = retrieved
                .get_first(self.fields.line)
                .and_then(|v| v.as_u64())
                .map(|l| l as u32);

            // Truncate snippet to a reasonable length for display.
            let snippet_text = truncate_snippet(snippet, 200);

            results.push(SearchResult {
                path: PathBuf::from(path),
                symbol,
                snippet: snippet_text,
                score: f64::from(score),
                source: SearchSource::Keyword,
                line,
            });
        }

        Ok(results)
    }

    /// Delete all documents and rebuild from the given set.
    pub fn rebuild(&self, docs: &[IndexDocument]) -> Result<()> {
        let mut writer: IndexWriter = self
            .index
            .writer(50_000_000)
            .context("failed to create index writer")?;

        writer.delete_all_documents().context("failed to clear index")?;
        writer.commit().context("failed to commit clear")?;

        for doc in docs {
            self.write_document(&mut writer, doc)?;
        }
        writer.commit().context("failed to commit rebuild")?;
        self.reader.reload().context("failed to reload reader")?;

        Ok(())
    }

    fn write_document(&self, writer: &mut IndexWriter, doc: &IndexDocument) -> Result<()> {
        let doc_type_str = match doc.doc_type {
            DocType::Code => "code",
            DocType::Wiki => "wiki",
        };

        let mut tantivy_doc = TantivyDocument::new();
        tantivy_doc.add_text(self.fields.path, doc.path.to_string_lossy());
        tantivy_doc.add_text(self.fields.content, &doc.content);
        tantivy_doc.add_text(self.fields.symbol, doc.symbol.as_deref().unwrap_or(""));
        tantivy_doc.add_text(self.fields.language, doc.language.as_deref().unwrap_or(""));
        tantivy_doc.add_text(self.fields.doc_type, doc_type_str);
        tantivy_doc.add_u64(self.fields.line, u64::from(doc.line.unwrap_or(0)));

        writer.add_document(tantivy_doc).context("failed to add document")?;
        Ok(())
    }
}

/// Truncate text to approximately `max_len` characters at a word boundary.
fn truncate_snippet(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }
    match text[..max_len].rfind(char::is_whitespace) {
        Some(pos) => format!("{}...", &text[..pos]),
        None => format!("{}...", &text[..max_len]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_index_and_search() {
        let idx = KeywordIndex::create_in_ram().unwrap();

        let docs = vec![
            IndexDocument {
                path: PathBuf::from("src/auth.rs"),
                content: "fn authenticate_user(token: &str) -> Result<User> { validate_jwt(token) }".to_string(),
                symbol: Some("authenticate_user".to_string()),
                language: Some("rust".to_string()),
                doc_type: DocType::Code,
                line: Some(42),
            },
            IndexDocument {
                path: PathBuf::from("src/db.rs"),
                content: "fn connect_database(url: &str) -> Pool { create_pool(url) }".to_string(),
                symbol: Some("connect_database".to_string()),
                language: Some("rust".to_string()),
                doc_type: DocType::Code,
                line: Some(10),
            },
            IndexDocument {
                path: PathBuf::from(".wiki/auth-middleware.md"),
                content: "# Authentication Middleware\nHandles JWT validation and user session management.".to_string(),
                symbol: None,
                language: None,
                doc_type: DocType::Wiki,
                line: None,
            },
        ];

        idx.add_documents(&docs).unwrap();

        let results = idx.search("authenticate jwt", 10).unwrap();
        assert!(!results.is_empty(), "expected results for 'authenticate jwt'");
        // The auth file should rank highest
        assert_eq!(results[0].path, PathBuf::from("src/auth.rs"));
        assert_eq!(results[0].source, SearchSource::Keyword);
    }

    #[test]
    fn test_keyword_empty_query() {
        let idx = KeywordIndex::create_in_ram().unwrap();
        let results = idx.search("", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_keyword_no_results() {
        let idx = KeywordIndex::create_in_ram().unwrap();
        let docs = vec![IndexDocument {
            path: PathBuf::from("src/main.rs"),
            content: "fn main() { println!(\"hello\"); }".to_string(),
            symbol: Some("main".to_string()),
            language: Some("rust".to_string()),
            doc_type: DocType::Code,
            line: Some(1),
        }];
        idx.add_documents(&docs).unwrap();

        let results = idx.search("authentication middleware jwt", 10).unwrap();
        assert!(results.is_empty(), "expected no results for unrelated query");
    }

    #[test]
    fn test_keyword_rebuild() {
        let idx = KeywordIndex::create_in_ram().unwrap();

        let docs_v1 = vec![IndexDocument {
            path: PathBuf::from("old.rs"),
            content: "old content about widgets".to_string(),
            symbol: None,
            language: None,
            doc_type: DocType::Code,
            line: None,
        }];
        idx.add_documents(&docs_v1).unwrap();

        let results = idx.search("widgets", 10).unwrap();
        assert!(!results.is_empty());

        // Rebuild with different content
        let docs_v2 = vec![IndexDocument {
            path: PathBuf::from("new.rs"),
            content: "new content about gadgets".to_string(),
            symbol: None,
            language: None,
            doc_type: DocType::Code,
            line: None,
        }];
        idx.rebuild(&docs_v2).unwrap();

        let results = idx.search("widgets", 10).unwrap();
        assert!(results.is_empty(), "old content should be gone after rebuild");

        let results = idx.search("gadgets", 10).unwrap();
        assert!(!results.is_empty(), "new content should be searchable");
    }
}
