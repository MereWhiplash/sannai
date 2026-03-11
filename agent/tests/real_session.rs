//! Test parsing a real Claude Code JSONL session file.
//!
//! Feeds every line from a captured session through the parser to find
//! crashes, unexpected errors, or event types we're not handling.

use sannai_agent::parser;
use std::fs;

#[test]
fn test_parse_real_session_file() {
    let content =
        fs::read_to_string("tests/fixtures/real_session.jsonl").expect("fixture file missing");

    let mut stats = Stats::default();

    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        stats.total_lines += 1;

        match parser::parse_line(line) {
            Ok(events) => {
                for event in &events {
                    match event {
                        parser::ParsedEvent::SessionStart { .. } => stats.session_starts += 1,
                        parser::ParsedEvent::UserPrompt { .. } => stats.user_prompts += 1,
                        parser::ParsedEvent::AssistantText { .. } => stats.assistant_texts += 1,
                        parser::ParsedEvent::ToolUse { tool_name, .. } => {
                            stats.tool_uses += 1;
                            *stats.tool_names.entry(tool_name.clone()).or_insert(0) += 1;
                        }
                        parser::ParsedEvent::ToolResult { is_error, .. } => {
                            stats.tool_results += 1;
                            if *is_error {
                                stats.tool_errors += 1;
                            }
                        }
                        parser::ParsedEvent::Ignored => stats.ignored += 1,
                    }
                }
            }
            Err(e) => {
                // Print the failing line for debugging
                eprintln!("ERROR on line {}: {}", i + 1, e);
                eprintln!("  Line preview: {}...", &line[..line.len().min(200)]);
                stats.parse_errors += 1;
            }
        }
    }

    // Print summary
    println!("\n=== Real Session Parse Results ===");
    println!("  Total lines:      {}", stats.total_lines);
    println!("  Parse errors:     {}", stats.parse_errors);
    println!("  Session starts:   {}", stats.session_starts);
    println!("  User prompts:     {}", stats.user_prompts);
    println!("  Assistant texts:  {}", stats.assistant_texts);
    println!("  Tool uses:        {}", stats.tool_uses);
    println!("  Tool results:     {}", stats.tool_results);
    println!("  Tool errors:      {}", stats.tool_errors);
    println!("  Ignored:          {}", stats.ignored);
    println!("  Tools used:");
    let mut tools: Vec<_> = stats.tool_names.iter().collect();
    tools.sort_by(|a, b| b.1.cmp(a.1));
    for (name, count) in &tools {
        println!("    {:<20} {}", name, count);
    }

    let total_events = stats.session_starts
        + stats.user_prompts
        + stats.assistant_texts
        + stats.tool_uses
        + stats.tool_results
        + stats.ignored;
    println!("  Total events:     {}", total_events);
    println!("=================================\n");

    // Assertions
    assert_eq!(
        stats.parse_errors, 0,
        "Parser should handle all lines without errors"
    );
    assert!(
        stats.user_prompts > 0,
        "Expected at least one user prompt"
    );
    assert!(
        stats.assistant_texts > 0,
        "Expected at least one assistant text"
    );
    assert!(
        stats.tool_uses > 0 || stats.ignored > 0,
        "Expected some tool uses or ignored events"
    );
}

#[derive(Default)]
struct Stats {
    total_lines: usize,
    parse_errors: usize,
    session_starts: usize,
    user_prompts: usize,
    assistant_texts: usize,
    tool_uses: usize,
    tool_results: usize,
    tool_errors: usize,
    ignored: usize,
    tool_names: std::collections::HashMap<String, usize>,
}
