use distri_search::DeepSearchAgent;
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("DeepSearch Agent - Basic Example");
    println!("==================================");
    
    // Create a simple test run of the DeepSearch agent
    let deep_search_agent = DeepSearchAgent::new();
    
    println!("✓ DeepSearch Agent created successfully!");
    println!("✓ This demonstrates the CustomAgent implementation");
    
    // Show the agent configuration
    println!("\nAgent Configuration:");
    println!("- Name: DeepSearch");
    println!("- Type: CustomAgent"); 
    println!("- Tools: mcp-tavily (search), mcp-spider (scrape)");
    println!("- Multi-step execution: Search → Scrape → Synthesize");
    
    println!("\nFor full functionality, you need:");
    println!("1. mcp-tavily server with TAVILY_API_KEY");
    println!("2. mcp-spider server"); 
    println!("3. Full distri framework setup");
    
    println!("\nSee README.md for complete setup instructions.");
    println!("See deep-search-config.yaml for configuration example.");
    
    Ok(())
}