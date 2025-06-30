use distri_search::{DeepSearchAgent, DeepSearchConfig};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("DeepSearch Agent - Basic Usage Example");
    println!("=====================================\n");

    // Create agent with default configuration
    let agent = DeepSearchAgent::new();
    
    // Demo 1: Show agent metadata
    println!("1. Agent Information:");
    println!("   Description: {}", agent.get_description());
    println!("   Required Tools: {:#}", serde_json::to_string_pretty(&agent.get_required_tools())?);
    println!();

    // Demo 2: Parse mock search results
    println!("2. Search Results Parsing:");
    let mock_search_response = json!({
        "results": [
            {
                "title": "AI Alignment Research 2024",
                "url": "https://example.com/ai-alignment",
                "content": "Recent developments in AI alignment include constitutional AI and RLHF improvements.",
                "score": 0.95
            },
            {
                "title": "Neural Network Safety",
                "url": "https://example.com/nn-safety", 
                "content": "New techniques for ensuring neural network safety in production environments.",
                "score": 0.87
            }
        ]
    });
    
    let search_results = agent.parse_search_results(&mock_search_response.to_string());
    println!("   Found {} search results:", search_results.len());
    for (i, result) in search_results.iter().enumerate() {
        println!("   {}. {} (Score: {:.2})", i+1, result.title, result.relevance_score);
        println!("      URL: {}", result.url);
        println!("      Snippet: {}", result.snippet);
    }
    println!();

    // Demo 3: URL selection for scraping
    println!("3. URL Selection for Scraping:");
    let urls_to_scrape = agent.select_urls_for_scraping(&search_results);
    println!("   Selected {} URLs for scraping:", urls_to_scrape.len());
    for (i, url) in urls_to_scrape.iter().enumerate() {
        println!("   {}. {}", i+1, url);
    }
    println!();

    // Demo 4: Parse mock scraped content
    println!("4. Scraped Content Parsing:");
    let mock_scraped_response = json!({
        "title": "AI Alignment Research 2024",
        "content": "Constitutional AI represents a major breakthrough in AI alignment research. This approach involves training AI systems to follow a set of principles..."
    });
    
    if let Some(scraped_content) = agent.parse_scraped_content(
        "https://example.com/ai-alignment",
        &mock_scraped_response.to_string()
    ) {
        println!("   Successfully parsed scraped content:");
        println!("   Title: {}", scraped_content.title);
        println!("   URL: {}", scraped_content.url);
        println!("   Summary: {}", scraped_content.summary);
    }
    println!();

    // Demo 5: Response synthesis
    println!("5. Response Synthesis:");
    let mock_scraped_content = vec![
        distri_search::ScrapedContent {
            url: "https://example.com/ai-alignment".to_string(),
            title: "AI Alignment Research 2024".to_string(),
            content: "Constitutional AI represents a major breakthrough...".to_string(),
            summary: "Constitutional AI represents a major breakthrough in AI alignment research.".to_string(),
        }
    ];
    
    let synthesized_response = agent.synthesize_response(
        "What are the latest developments in AI alignment?",
        &search_results,
        &mock_scraped_content
    );
    
    println!("   Generated comprehensive response:");
    println!("{}", synthesized_response);
    println!();

    // Demo 6: Custom configuration
    println!("6. Custom Configuration:");
    let custom_config = DeepSearchConfig {
        max_search_results: 10,
        max_scrape_urls: 5,
        search_timeout: 60,
        scrape_timeout: 45,
    };
    
    let custom_agent = DeepSearchAgent::with_config(custom_config);
    println!("   Created custom agent with:");
    println!("   - Max search results: {}", custom_agent.config.max_search_results);
    println!("   - Max scrape URLs: {}", custom_agent.config.max_scrape_urls);
    println!("   - Search timeout: {}s", custom_agent.config.search_timeout);
    println!("   - Scrape timeout: {}s", custom_agent.config.scrape_timeout);
    println!();

    // Demo 7: Tool configurations
    println!("7. MCP Tool Configurations:");
    let search_config = agent.create_search_config("AI alignment research");
    println!("   Search config: {}", serde_json::to_string_pretty(&search_config)?);
    
    let scrape_config = agent.create_scrape_config("https://example.com/article");
    println!("   Scrape config: {}", serde_json::to_string_pretty(&scrape_config)?);

    println!("\n✓ All examples completed successfully!");
    println!("   For full integration, use the 'full' feature and distri framework.");
    
    Ok(())
}