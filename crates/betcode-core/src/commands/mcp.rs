//! Extraction of MCP tool entries from Claude Code's `system_init` tools list.

use crate::ndjson::ToolSchema;

use super::{CommandCategory, CommandEntry, ExecutionMode};

/// Converts MCP tool schemas from `system_init` into command entries.
///
/// Only tools with names matching the `mcp__server__tool` convention are included.
pub fn mcp_tools_to_entries(tools: &[ToolSchema]) -> Vec<CommandEntry> {
    tools
        .iter()
        .filter_map(|tool| {
            let rest = tool.name.strip_prefix("mcp__")?;
            let (server, tool_name) = rest.split_once("__")?;
            if server.is_empty() || tool_name.is_empty() {
                return None;
            }
            let display_name = format!("{server}:{tool_name}");
            let description = tool
                .description
                .clone()
                .unwrap_or_else(|| format!("MCP tool: {tool_name}"));
            Some(
                CommandEntry::new(
                    &display_name,
                    &description,
                    CommandCategory::Mcp,
                    ExecutionMode::Passthrough,
                    "mcp",
                )
                .with_group(server)
                .with_display_name(&display_name),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(name: &str, desc: Option<&str>) -> ToolSchema {
        ToolSchema {
            name: name.to_string(),
            description: desc.map(String::from),
            input_schema: None,
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_extracts_mcp_tools() {
        let tools = vec![
            make_tool(
                "mcp__chrome-devtools__take_screenshot",
                Some("Take a screenshot"),
            ),
            make_tool("mcp__chrome-devtools__click", Some("Click an element")),
            make_tool("mcp__tavily__tavily-search", Some("Search the web")),
            make_tool("Read", Some("Read a file")),
            make_tool("Write", Some("Write a file")),
        ];

        let entries = mcp_tools_to_entries(&tools);

        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| e.category == CommandCategory::Mcp));

        let chrome = entries
            .iter()
            .find(|e| e.name == "chrome-devtools:take_screenshot");
        assert!(chrome.is_some());
        let chrome = chrome.unwrap();
        assert_eq!(chrome.group.as_deref(), Some("chrome-devtools"));
        assert_eq!(chrome.source, "mcp");

        let tavily = entries.iter().find(|e| e.name == "tavily:tavily-search");
        assert!(tavily.is_some());
        assert_eq!(tavily.unwrap().group.as_deref(), Some("tavily"));
    }

    #[test]
    fn test_no_mcp_tools() {
        let tools = vec![make_tool("Read", None), make_tool("Bash", None)];
        let entries = mcp_tools_to_entries(&tools);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_malformed_mcp_name_skipped() {
        let tools = vec![
            make_tool("mcp__", None),
            make_tool("mcp__server", None),
            make_tool("mcp__server__tool", Some("OK")),
        ];
        let entries = mcp_tools_to_entries(&tools);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "server:tool");
    }
}
