//! MCP `Tool` catalog — the callable functions the agent can invoke.
//!
//! v0.19 wave 1 ships the descriptors. Wave 2 wires them to the real
//! implementation (delegating to existing CLI commands).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub kind: ToolKind,
    pub input_schema_json: String,
    /// Read-only tools work in `--read-only` mode (the default). Write
    /// tools require `--allow-write-tools` per PRD risk #25.
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Query,          // LLM-augmented (delegates to coral query)
    Search,         // TF-IDF (delegates to coral search)
    FindBacklinks,  // pure
    AffectedRepos,  // delegates to filters --since
    Verify,         // delegates to coral verify
    ListInterfaces, // list Interface-typed pages (v0.24 M2.2)
    ContractStatus, // latest contract drift check results (v0.24 M2.2)
    RunTest,        // delegates to coral test (write tool)
    Up,             // delegates to coral up (write tool)
    Down,           // delegates to coral down (write tool)
    // v0.41 code-intelligence tools (coral-graph / coral-index / coral-search)
    SymbolLookup,   // find symbol definition + signature + location
    Callers,        // list all callers of a symbol
    Callees,        // list all symbols called by a function
    References,     // find all references to a symbol
    AffectedBy,     // files/symbols affected by a change to a file
    FileSymbols,    // list all symbols defined in a file
    SemanticSearch, // hybrid search across code and wiki
    DetectPatterns, // list coding patterns detected in the repo
    // v0.42 implementation tools
    PlanChange,     // generate implementation plan
    GenerateCode,   // generate code sketch (diff preview)
    ValidateChange, // validate changes (write tool)
}

pub struct ToolCatalog;

impl ToolCatalog {
    pub fn read_only() -> Vec<Tool> {
        vec![
            Tool {
                name: "query".into(),
                description: "Streamed LLM answer using the wiki as context. Cites slugs.".into(),
                kind: ToolKind::Query,
                input_schema_json: r#"{"type":"object","properties":{"q":{"type":"string"},"repo":{"type":"string"},"tag":{"type":"string"}},"required":["q"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "search".into(),
                description: "TF-IDF full-text search across the wiki. No LLM.".into(),
                kind: ToolKind::Search,
                input_schema_json: r#"{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "find_backlinks".into(),
                description: "Return all wiki slugs that link to a given slug.".into(),
                kind: ToolKind::FindBacklinks,
                input_schema_json: r#"{"type":"object","properties":{"slug":{"type":"string"}},"required":["slug"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "affected_repos".into(),
                description: "List repos changed since a given SHA, including downstream dependents.".into(),
                kind: ToolKind::AffectedRepos,
                input_schema_json: r#"{"type":"object","properties":{"since":{"type":"string"}},"required":["since"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "verify".into(),
                description: "Run liveness healthchecks against the running environment.".into(),
                kind: ToolKind::Verify,
                input_schema_json: r#"{"type":"object","properties":{"env":{"type":"string"}}}"#.into(),
                read_only: true,
            },
            Tool {
                name: "list_interfaces".into(),
                description: "List all interface/contract pages in the wiki.".into(),
                kind: ToolKind::ListInterfaces,
                input_schema_json: r#"{"type":"object","properties":{}}"#.into(),
                read_only: true,
            },
            Tool {
                name: "contract_status".into(),
                description: "Return the latest contract drift check results.".into(),
                kind: ToolKind::ContractStatus,
                input_schema_json: r#"{"type":"object","properties":{"repo":{"type":"string"}}}"#.into(),
                read_only: true,
            },
            // v0.41 code-intelligence tools
            Tool {
                name: "symbol_lookup".into(),
                description: "Find a symbol by name. Returns its definition, signature, and source location.".into(),
                kind: ToolKind::SymbolLookup,
                input_schema_json: r#"{"type":"object","properties":{"symbol":{"type":"string","description":"Fully-qualified or short symbol name to look up"}},"required":["symbol"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "callers".into(),
                description: "List all callers of a symbol (incoming call graph edges).".into(),
                kind: ToolKind::Callers,
                input_schema_json: r#"{"type":"object","properties":{"symbol":{"type":"string","description":"Symbol name to find callers of"}},"required":["symbol"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "callees".into(),
                description: "List all symbols called by a function (outgoing call graph edges).".into(),
                kind: ToolKind::Callees,
                input_schema_json: r#"{"type":"object","properties":{"symbol":{"type":"string","description":"Function name to find callees of"}},"required":["symbol"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "references".into(),
                description: "Find all references to a symbol across the codebase.".into(),
                kind: ToolKind::References,
                input_schema_json: r#"{"type":"object","properties":{"symbol":{"type":"string","description":"Symbol name to find references to"}},"required":["symbol"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "affected_by".into(),
                description: "List files and symbols affected by a change to a file, transitively up to a given depth.".into(),
                kind: ToolKind::AffectedBy,
                input_schema_json: r#"{"type":"object","properties":{"file":{"type":"string","description":"Path to the changed file"},"depth":{"type":"number","description":"Max transitive depth (default: 2)"}},"required":["file"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "file_symbols".into(),
                description: "List all symbols defined in a file with their signatures and line numbers.".into(),
                kind: ToolKind::FileSymbols,
                input_schema_json: r#"{"type":"object","properties":{"file":{"type":"string","description":"Path to the file to inspect"}},"required":["file"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "semantic_search".into(),
                description: "Hybrid semantic + keyword search across code and wiki content.".into(),
                kind: ToolKind::SemanticSearch,
                input_schema_json: r#"{"type":"object","properties":{"query":{"type":"string","description":"Natural-language search query"},"limit":{"type":"number","description":"Max results to return (default: 10)"},"source":{"type":"string","description":"Filter by source: 'code', 'wiki', or 'all' (default: 'all')"}},"required":["query"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "detect_patterns".into(),
                description: "Detect and list coding patterns found in the repository (e.g. error handling, DI, middleware chains).".into(),
                kind: ToolKind::DetectPatterns,
                input_schema_json: r#"{"type":"object","properties":{}}"#.into(),
                read_only: true,
            },
            // v0.42 implementation tools (read-only)
            Tool {
                name: "plan_change".into(),
                description: "Generate a structured implementation plan from a natural language description. Returns YAML plan with steps, impact, and risks.".into(),
                kind: ToolKind::PlanChange,
                input_schema_json: r#"{"type":"object","properties":{"description":{"type":"string","description":"Natural language description of the change to implement"},"context":{"type":"object","description":"Optional context: relevant_code, relevant_wiki, patterns, affected_files"}},"required":["description"]}"#.into(),
                read_only: true,
            },
            Tool {
                name: "generate_code".into(),
                description: "Generate a code sketch (unified diff preview) from a natural language description.".into(),
                kind: ToolKind::GenerateCode,
                input_schema_json: r#"{"type":"object","properties":{"description":{"type":"string","description":"Natural language description of the code to generate"},"files":{"type":"array","items":{"type":"string"},"description":"Paths to relevant source files for context"}},"required":["description"]}"#.into(),
                read_only: true,
            },
        ]
    }

    pub fn write() -> Vec<Tool> {
        vec![
            Tool {
                name: "run_test".into(),
                description: "Run a specific test case by id against the running environment.".into(),
                kind: ToolKind::RunTest,
                input_schema_json: r#"{"type":"object","properties":{"case_id":{"type":"string"}},"required":["case_id"]}"#.into(),
                read_only: false,
            },
            Tool {
                name: "up".into(),
                description: "Bring up the dev environment (compose up).".into(),
                kind: ToolKind::Up,
                input_schema_json: r#"{"type":"object","properties":{"env":{"type":"string"},"watch":{"type":"boolean"}}}"#.into(),
                read_only: false,
            },
            Tool {
                name: "down".into(),
                description: "Tear down the dev environment.".into(),
                kind: ToolKind::Down,
                input_schema_json: r#"{"type":"object","properties":{"env":{"type":"string"},"volumes":{"type":"boolean"}}}"#.into(),
                read_only: false,
            },
            // v0.42 implementation tool (write)
            Tool {
                name: "validate_change".into(),
                description: "Validate changed files: check existence, compilation, and tests. Returns pass/fail with details.".into(),
                kind: ToolKind::ValidateChange,
                input_schema_json: r#"{"type":"object","properties":{"files":{"type":"array","items":{"type":"string"},"description":"List of changed file paths to validate"},"root":{"type":"string","description":"Project root directory (default: current directory)"}},"required":["files"]}"#.into(),
                read_only: false,
            },
        ]
    }

    pub fn all() -> Vec<Tool> {
        let mut all = Self::read_only();
        all.extend(Self::write());
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_tools_are_all_marked_read_only() {
        for t in ToolCatalog::read_only() {
            assert!(t.read_only, "{} should be read-only", t.name);
        }
    }

    #[test]
    fn write_tools_are_not_marked_read_only() {
        for t in ToolCatalog::write() {
            assert!(!t.read_only, "{} should not be read-only", t.name);
        }
    }

    #[test]
    fn all_returns_full_catalog() {
        let all = ToolCatalog::all();
        assert_eq!(
            all.len(),
            ToolCatalog::read_only().len() + ToolCatalog::write().len()
        );
    }

    #[test]
    fn input_schemas_are_valid_json() {
        for t in ToolCatalog::all() {
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&t.input_schema_json);
            assert!(
                parsed.is_ok(),
                "tool {} has invalid schema: {}",
                t.name,
                t.input_schema_json
            );
        }
    }

    /// v0.24 M2.2: `ListInterfaces` serializes to `list_interfaces`.
    #[test]
    fn list_interfaces_kind_serializes_correctly() {
        let json = serde_json::to_string(&ToolKind::ListInterfaces).unwrap();
        assert_eq!(json, r#""list_interfaces""#);
    }

    /// v0.24 M2.2: `ContractStatus` serializes to `contract_status`.
    #[test]
    fn contract_status_kind_serializes_correctly() {
        let json = serde_json::to_string(&ToolKind::ContractStatus).unwrap();
        assert_eq!(json, r#""contract_status""#);
    }

    /// v0.24 M2.2: read-only catalog includes the two contract tools.
    #[test]
    fn read_only_includes_contract_tools() {
        let names: Vec<String> = ToolCatalog::read_only()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(names.contains(&"list_interfaces".to_string()));
        assert!(names.contains(&"contract_status".to_string()));
    }

    /// v0.41: code-intelligence tools are present in the read-only catalog.
    #[test]
    fn read_only_includes_code_intelligence_tools() {
        let names: Vec<String> = ToolCatalog::read_only()
            .into_iter()
            .map(|t| t.name)
            .collect();
        let expected = [
            "symbol_lookup",
            "callers",
            "callees",
            "references",
            "affected_by",
            "file_symbols",
            "semantic_search",
            "detect_patterns",
        ];
        for name in expected {
            assert!(
                names.contains(&name.to_string()),
                "read-only catalog missing tool: {name}"
            );
        }
    }

    /// v0.41: code-intelligence ToolKind variants serialize to snake_case.
    #[test]
    fn code_intelligence_kinds_serialize_correctly() {
        let cases = [
            (ToolKind::SymbolLookup, "symbol_lookup"),
            (ToolKind::Callers, "callers"),
            (ToolKind::Callees, "callees"),
            (ToolKind::References, "references"),
            (ToolKind::AffectedBy, "affected_by"),
            (ToolKind::FileSymbols, "file_symbols"),
            (ToolKind::SemanticSearch, "semantic_search"),
            (ToolKind::DetectPatterns, "detect_patterns"),
        ];
        for (kind, expected) in cases {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{expected}\""), "ToolKind::{kind:?}");
        }
    }

    /// v0.42: implementation tool kinds serialize to snake_case.
    #[test]
    fn implementation_tool_kinds_serialize_correctly() {
        let cases = [
            (ToolKind::PlanChange, "plan_change"),
            (ToolKind::GenerateCode, "generate_code"),
            (ToolKind::ValidateChange, "validate_change"),
        ];
        for (kind, expected) in cases {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{expected}\""), "ToolKind::{kind:?}");
        }
    }

    /// v0.42: read-only catalog includes the implementation tools.
    #[test]
    fn read_only_includes_implementation_tools() {
        let names: Vec<String> = ToolCatalog::read_only()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(names.contains(&"plan_change".to_string()));
        assert!(names.contains(&"generate_code".to_string()));
    }

    /// v0.42: validate_change is a write tool, not in read-only catalog.
    #[test]
    fn validate_change_is_write_tool() {
        let read_names: Vec<String> = ToolCatalog::read_only()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(!read_names.contains(&"validate_change".to_string()));

        let write_names: Vec<String> = ToolCatalog::write()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(write_names.contains(&"validate_change".to_string()));
    }

    /// v0.41: code-intelligence tool schemas are valid JSON with "type":"object".
    #[test]
    fn code_intelligence_schemas_are_valid_object_schemas() {
        let ci_tools: Vec<Tool> = ToolCatalog::read_only()
            .into_iter()
            .filter(|t| {
                matches!(
                    t.kind,
                    ToolKind::SymbolLookup
                        | ToolKind::Callers
                        | ToolKind::Callees
                        | ToolKind::References
                        | ToolKind::AffectedBy
                        | ToolKind::FileSymbols
                        | ToolKind::SemanticSearch
                        | ToolKind::DetectPatterns
                )
            })
            .collect();
        assert_eq!(ci_tools.len(), 8, "expected 8 code-intelligence tools");
        for t in &ci_tools {
            let v: serde_json::Value = serde_json::from_str(&t.input_schema_json)
                .unwrap_or_else(|e| panic!("tool {} schema parse error: {e}", t.name));
            assert_eq!(
                v.get("type").and_then(|v| v.as_str()),
                Some("object"),
                "tool {} schema must have type:object",
                t.name
            );
            assert!(
                v.get("properties").is_some(),
                "tool {} schema must have properties",
                t.name
            );
        }
    }
}
