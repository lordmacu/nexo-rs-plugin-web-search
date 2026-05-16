//! `web_search` tool descriptor advertised at handshake.

use nexo_microapp_sdk::plugin::ToolDef;
use serde_json::json;

pub fn tool_defs() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "web_search".into(),
        description: "Search the web. Returns titles, URLs, and snippets from a \
            configured provider (Brave / Tavily / DuckDuckGo / Perplexity). \
            Multi-instance: when the operator declares multiple search profiles \
            in `web-search.yaml::instances[]`, pass the optional `instance:` \
            arg to target a specific profile; absent uses the calling agent's \
            default. Provider can be overridden per call via `provider:`."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query":     { "type": "string",  "description": "Search query string." },
                "count":     { "type": "integer", "minimum": 1, "maximum": 10, "description": "Number of results (1-10). Default from per-binding policy." },
                "instance":  { "type": "string",  "description": "Optional search-profile id declared in web-search.yaml::instances[]. Absent → agent's default profile." },
                "provider":  { "type": "string",  "enum": ["brave","tavily","duckduckgo","perplexity"], "description": "Override provider for this call." },
                "freshness": { "type": "string",  "enum": ["day","week","month","year"], "description": "Time window filter." },
                "country":   { "type": "string",  "description": "ISO-3166 alpha-2 country code." },
                "language":  { "type": "string",  "description": "ISO-639-1 language code." },
                "expand":    { "type": "boolean", "description": "Reserved for v0.2.0; accepted but no-op in v0.1.0." }
            },
            "required": ["query"]
        }),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defines_one_tool() {
        let defs = tool_defs();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "web_search");
    }

    #[test]
    fn web_search_schema_requires_query() {
        let defs = tool_defs();
        let def = &defs[0];
        let required = def.input_schema["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(names, vec!["query"]);
    }

    #[test]
    fn schema_documents_instance_arg_for_multi_instance() {
        let defs = tool_defs();
        let props = &defs[0].input_schema["properties"];
        assert!(props.get("instance").is_some(), "missing `instance` arg");
    }
}
