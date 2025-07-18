# Distri-Scraper

A powerful AI-powered web scraping agent built with the Distri framework that leverages code agents for programmatic data extraction from websites.

## Overview

The Distri-Scraper is an intelligent web scraping agent that can:

- **Programmatically scrape websites** using CSS selectors and XPath queries
- **Handle dynamic content** rendered by JavaScript
- **Extract structured data** in various formats (JSON, CSV, XML)
- **Follow links and paginate** through content systematically
- **Manage sessions and cookies** for authenticated scraping
- **Respect rate limits** and website terms of service
- **Provide detailed error handling** and retry mechanisms

## Features

### Core Scraping Capabilities
- HTTP request handling with custom headers and user agents
- HTML parsing and DOM traversal
- CSS selector-based element extraction
- XPath query support for complex data extraction
- JavaScript rendering for Single Page Applications (SPAs)
- Form submission and interaction handling

### Data Processing
- Structured data extraction to JSON, CSV, and other formats
- Data cleaning and normalization
- Duplicate detection and removal
- Data validation and type conversion

### Advanced Features
- Session management for authenticated sites
- Cookie handling and persistence
- Rate limiting and request throttling
- Proxy support for IP rotation
- Retry logic with exponential backoff
- Robots.txt compliance checking

## Usage

### Building the Agent

```bash
cd samples/distri-scraper
cargo build --release
```

### Running the Agent

#### Interactive Chat Mode
```bash
cargo run --bin scraper -- chat --agent distri-scraper
```

#### Background Agent Mode
```bash
cargo run --bin scraper -- background --agent distri-scraper
```

#### Session Mode
```bash
cargo run --bin scraper -- session --agent distri-scraper --session-id my-scraping-session
```

### Example Commands

Once the agent is running, you can give it various scraping tasks:

#### Basic Web Scraping
```
"Scrape all product titles and prices from https://example-ecommerce.com/products"
```

#### Structured Data Extraction
```
"Extract all job postings from https://jobs.example.com including title, company, location, and salary, and format as JSON"
```

#### Paginated Content
```
"Scrape all articles from https://news.example.com, following pagination links up to 10 pages"
```

#### JavaScript-Heavy Sites
```
"Scrape data from https://spa-example.com which loads content dynamically, wait for the 'loaded' class to appear"
```

#### Complex Data Processing
```
"Scrape product reviews from https://reviews.example.com, extract sentiment, rating, and review text, then generate a summary report"
```

## Configuration

### Environment Variables

Create a `.env` file in the project root:

```env
# Optional: Tavily API key for enhanced research capabilities
TAVILY_API_KEY=your_tavily_api_key_here

# Optional: Proxy settings
HTTP_PROXY=http://proxy.example.com:8080
HTTPS_PROXY=https://proxy.example.com:8080

# Optional: Rate limiting settings
DEFAULT_DELAY_MS=1000
MAX_CONCURRENT_REQUESTS=5
```

### Agent Configuration

The agent behavior can be customized in `definition.yaml`:

```yaml
agents:
  - name: "distri-scraper"
    agent_type: "code"
    model_settings:
      model: "gpt-4.1"
      temperature: 0.3  # Lower temperature for more consistent scraping
      max_tokens: 4000
    max_iterations: 15  # Allow for complex multi-step scraping tasks
```

## Dependencies

The scraper relies on the following MCP servers:

- **mcp-spider**: Core web scraping capabilities
- **mcp-tavily**: Enhanced web search and research features

## Ethical Considerations

### Responsible Scraping
- Always check and respect `robots.txt` files
- Implement appropriate delays between requests
- Respect website terms of service
- Use reasonable concurrency limits
- Handle errors gracefully without overwhelming servers

### Legal Compliance
- Ensure compliance with website terms of service
- Respect copyright and intellectual property rights
- Follow data protection regulations (GDPR, CCPA, etc.)
- Obtain necessary permissions for commercial use

### Best Practices
- Use descriptive user agents
- Cache responses when appropriate
- Implement circuit breakers for failing sites
- Monitor and log scraping activities
- Provide attribution when required

## Examples

### E-commerce Price Monitoring
```rust
// The agent can be programmed to:
// 1. Navigate to product pages
// 2. Extract current prices
// 3. Compare with historical data
// 4. Generate price trend reports
```

### News Article Aggregation
```rust
// The agent can:
// 1. Scrape multiple news sources
// 2. Extract article metadata
// 3. Deduplicate similar stories
// 4. Generate content summaries
```

### Job Market Analysis
```rust
// The agent can:
// 1. Scrape job listings from multiple boards
// 2. Extract job requirements and skills
// 3. Analyze salary trends
// 4. Generate market insights
```

## Troubleshooting

### Common Issues

1. **Rate Limiting**: Increase delays between requests
2. **JavaScript Content**: Ensure JavaScript rendering is enabled
3. **Authentication**: Check session management configuration
4. **Blocked Requests**: Consider proxy rotation or user agent changes

### Debug Mode

Enable verbose logging:
```bash
RUST_LOG=debug cargo run --bin scraper -- chat --agent distri-scraper
```

## Contributing

Contributions are welcome! Please ensure all scraping activities:
- Respect website terms of service
- Include appropriate rate limiting
- Handle errors gracefully
- Include comprehensive tests

## License

This project is licensed under the same terms as the main Distri project.