#[cfg(test)]
mod tests {
    use distri_search::{DeepSearchAgent, DeepSearchConfig, SearchResult, ScrapedContent};
    use serde_json::json;

    #[test]
    fn test_agent_creation() {
        let agent = DeepSearchAgent::new();
        assert_eq!(agent.config.max_search_results, 5);
        assert_eq!(agent.config.max_scrape_urls, 3);
        
        let custom_config = DeepSearchConfig {
            max_search_results: 10,
            max_scrape_urls: 5,
            search_timeout: 60,
            scrape_timeout: 45,
        };
        
        let custom_agent = DeepSearchAgent::with_config(custom_config);
        assert_eq!(custom_agent.config.max_search_results, 10);
        assert_eq!(custom_agent.config.max_scrape_urls, 5);
    }

    #[test]
    fn test_search_results_parsing() {
        let agent = DeepSearchAgent::new();
        
        let search_response = json!({
            "results": [
                {
                    "title": "Test Article",
                    "url": "https://example.com/test",
                    "content": "Test content snippet",
                    "score": 0.9
                }
            ]
        });
        
        let results = agent.parse_search_results(&search_response.to_string());
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Test Article");
        assert_eq!(results[0].url, "https://example.com/test");
        assert_eq!(results[0].relevance_score, 0.9);
    }

    #[test]
    fn test_url_selection() {
        let agent = DeepSearchAgent::new();
        
        let search_results = vec![
            SearchResult {
                title: "High Score Article".to_string(),
                url: "https://example.com/high".to_string(),
                snippet: "High relevance content".to_string(),
                relevance_score: 0.95,
            },
            SearchResult {
                title: "Medium Score Article".to_string(),
                url: "https://example.com/medium".to_string(),
                snippet: "Medium relevance content".to_string(),
                relevance_score: 0.7,
            },
            SearchResult {
                title: "Low Score Article".to_string(),
                url: "https://example.com/low".to_string(),
                snippet: "Low relevance content".to_string(),
                relevance_score: 0.4,
            },
        ];
        
        let selected_urls = agent.select_urls_for_scraping(&search_results);
        
        // Should select top 3 URLs by score
        assert_eq!(selected_urls.len(), 3);
        assert_eq!(selected_urls[0], "https://example.com/high");
        assert_eq!(selected_urls[1], "https://example.com/medium");
        assert_eq!(selected_urls[2], "https://example.com/low");
    }

    #[test]
    fn test_scraped_content_parsing() {
        let agent = DeepSearchAgent::new();
        
        let scraped_response = json!({
            "title": "Test Page",
            "content": "This is the full content of the test page with lots of information."
        });
        
        let parsed = agent.parse_scraped_content(
            "https://example.com/test",
            &scraped_response.to_string()
        );
        
        assert!(parsed.is_some());
        let content = parsed.unwrap();
        assert_eq!(content.title, "Test Page");
        assert_eq!(content.url, "https://example.com/test");
        assert!(content.content.contains("full content"));
    }

    #[test]
    fn test_response_synthesis() {
        let agent = DeepSearchAgent::new();
        
        let search_results = vec![
            SearchResult {
                title: "Test Article".to_string(),
                url: "https://example.com/test".to_string(),
                snippet: "Test snippet".to_string(),
                relevance_score: 0.9,
            }
        ];
        
        let scraped_content = vec![
            ScrapedContent {
                url: "https://example.com/test".to_string(),
                title: "Test Article".to_string(),
                content: "Full article content here...".to_string(),
                summary: "Article summary here...".to_string(),
            }
        ];
        
        let response = agent.synthesize_response(
            "test query",
            &search_results,
            &scraped_content
        );
        
        assert!(response.contains("DeepSearch Results for: test query"));
        assert!(response.contains("Search Overview"));
        assert!(response.contains("Detailed Analysis"));
        assert!(response.contains("Test Article"));
        assert!(response.contains("Summary"));
    }

    #[test]
    fn test_tool_configurations() {
        let agent = DeepSearchAgent::new();
        
        let search_config = agent.create_search_config("test query");
        assert_eq!(search_config["tool_name"], "search");
        assert_eq!(search_config["input"]["query"], "test query");
        assert_eq!(search_config["input"]["max_results"], 5);
        
        let scrape_config = agent.create_scrape_config("https://example.com");
        assert_eq!(scrape_config["tool_name"], "scrape");
        assert_eq!(scrape_config["input"]["url"], "https://example.com");
        assert_eq!(scrape_config["input"]["include_links"], false);
    }

    #[test]
    fn test_agent_metadata() {
        let agent = DeepSearchAgent::new();
        
        let description = agent.get_description();
        assert!(description.contains("intelligent research agent"));
        
        let system_prompt = agent.get_system_prompt();
        assert!(system_prompt.contains("DeepSearch"));
        assert!(system_prompt.contains("search"));
        assert!(system_prompt.contains("scrape"));
        
        let required_tools = agent.get_required_tools();
        assert_eq!(required_tools.len(), 2);
        
        // Check mcp-tavily
        let tavily_tool = &required_tools[0];
        assert_eq!(tavily_tool["name"], "mcp-tavily");
        assert_eq!(tavily_tool["type"], "tool");
        
        // Check mcp-spider
        let spider_tool = &required_tools[1];
        assert_eq!(spider_tool["name"], "mcp-spider");
        assert_eq!(spider_tool["type"], "tool");
    }

    #[cfg(feature = "full")]
    #[test]
    fn test_custom_agent_trait() {
        // This test would only run when the 'full' feature is enabled
        // and would test the CustomAgent implementation
        use distri::agent::CustomAgent;
        
        let agent = DeepSearchAgent::new();
        let cloned = agent.clone_box();
        
        // Verify the agent implements CustomAgent trait
        // In a real integration test, you would test the step() method
        // with mock messages and verify tool calls and responses
        assert!(std::ptr::addr_of!(*cloned) != std::ptr::addr_of!(agent));
    }
}