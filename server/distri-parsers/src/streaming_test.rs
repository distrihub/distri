//! Test streaming functionality with multiple text blocks

#[cfg(test)]
mod tests {
    use distri_types::ToolCallFormat;

    use crate::ParserFactory;

    #[test]
    fn test_xml_streaming_with_multiple_chunks() {
        // Test XML streaming with multiple chunks - explicitly pass tool names for testing
        let test_tool_names = vec!["search".to_string(), "final".to_string()];
        let mut parser =
            ParserFactory::create_parser(&ToolCallFormat::Xml, test_tool_names).unwrap();

        // Simulate streaming chunks that build up a complete tool call
        let chunks = vec![
            "<sea",
            "rch>\n<qu",
            "ery>test query</",
            "query>\n<lim",
            "it>5</limit>\n</se",
            "arch>\n\n<fin",
            "al>\n<mess",
            "age>All done!</mes",
            "sage>\n</final>",
        ];

        println!("Testing XML streaming with multiple chunks...");
        let mut total_tool_calls = 0;

        // Let's also build up the content step by step to see when tool calls should complete
        let mut accumulated_content = String::new();

        for (i, chunk) in chunks.iter().enumerate() {
            accumulated_content.push_str(chunk);
            println!("Processing chunk {}: '{}'", i + 1, chunk);
            println!("  Accumulated so far: '{}'", accumulated_content);

            match parser.process_chunk(chunk) {
                Ok(result) => {
                    println!("  New tool calls: {}", result.new_tool_calls.len());
                    total_tool_calls += result.new_tool_calls.len();

                    if !result.new_tool_calls.is_empty() {
                        for tool_call in &result.new_tool_calls {
                            println!("    - {}: {:?}", tool_call.tool_name, tool_call.input);
                        }
                    }

                    println!("  Has partial tool call: {}", result.has_partial_tool_call);

                    // After chunk 5, we should have the complete search tool call
                    if i == 4 {
                        // chunk 5 (0-indexed)
                        println!(
                            "  *** After chunk 5, we should have complete search tool call ***"
                        );
                    }
                }
                Err(e) => {
                    println!("  Error: {}", e);
                    panic!("Streaming should not fail: {}", e);
                }
            }
            println!();
        }

        // Finalize to get any remaining tool calls
        println!("Finalizing parser...");
        match parser.finalize() {
            Ok(final_tool_calls) => {
                println!("Final tool calls from finalize: {}", final_tool_calls.len());
                total_tool_calls += final_tool_calls.len();
            }
            Err(e) => {
                println!("Finalize error: {}", e);
            }
        }

        // We should have exactly 2 tool calls: search and final
        assert_eq!(
            total_tool_calls, 2,
            "Expected 2 tool calls total from streaming"
        );
        println!(
            "✅ Streaming test passed! Total tool calls: {}",
            total_tool_calls
        );
    }
}
