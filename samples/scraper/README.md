# Distri Scraper Samples

This directory contains various configurations for the Distri Scraper agent, demonstrating different strategies and approaches to web scraping.

## Configuration Structure

The samples use YAML inheritance to avoid duplication and make configuration management easier:

```
distri-scraper/
├── base-config.yaml          # Base configuration with common settings
├── definition.yaml           # Main scraper configuration
├── definition-default.yaml   # Default strategy (extends base-config.yaml)
├── definition-cot.yaml       # Chain of Thought strategy
├── definition-react.yaml     # ReAct strategy
├── definition-approval.yaml  # Approval-based strategy
└── README.md                # This file
```

## YAML Inheritance

All configuration files can extend a base configuration using the `extends` field:

```yaml
# Child configuration
extends: "base-config.yaml"

agents:
  - name: "my-agent"
    # Override or add specific settings
    strategy:
      type: "react"
```

### How Inheritance Works

1. **Base Configuration**: Common settings like tools, servers, and basic agent properties
2. **Strategy-Specific Configs**: Override strategy type and parameters
3. **Merging**: Child configurations override parent settings, with deep merging for nested objects

## Available Configurations

### Base Configuration (`base-config.yaml`)
- Common agent properties
- Tool definitions
- Server configurations
- Memory and rejection handling

### Main Scraper (`definition.yaml`)
- Complete web scraping agent
- Uses plan-and-execute strategy
- Includes search and spider tools

### Strategy Variants

#### Default Strategy (`definition-default.yaml`)
- Extends base configuration
- Uses simple sequential execution
- Good for straightforward scraping tasks

#### Chain of Thought (`definition-cot.yaml`)
- Extends base configuration
- Uses CoT strategy for complex reasoning
- Better for tasks requiring step-by-step analysis

#### ReAct Strategy (`definition-react.yaml`)
- Extends base configuration
- Uses ReAct (Reasoning + Acting) strategy
- Good for interactive exploration and problem-solving

#### Approval Strategy (`definition-approval.yaml`)
- Extends base configuration
- Requires user approval for sensitive operations
- Enhanced security for production use

## Usage Examples

### Basic Usage
```bash
# Use main scraper configuration
cargo run --bin distri run --config samples/distri-scraper/definition.yaml --task "Scrape news from CNN"

# Use specific strategy
cargo run --bin distri run --config samples/distri-scraper/definition-react.yaml --task "Find product prices on Amazon"
```

### With CLI Overrides
```bash
# Override strategy type
cargo run --bin distri run \
  --config samples/distri-scraper/definition.yaml \
  --override-config "agents[0].strategy.type=react" \
  --task "Scrape data from a website"

# Override model settings
cargo run --bin distri run \
  --config samples/distri-scraper/definition.yaml \
  --override-config "agents[0].model_settings.model=gpt-4-turbo" \
  --override-config "agents[0].model_settings.temperature=0.1" \
  --task "Extract structured data from a table"
```

## Configuration Features

### Environment Variable Support
```yaml
servers:
  - name: "tavily"
    config:
      api_key: "{{TAVILY_API_KEY}}"  # Will be replaced with env var
```

### Deep Merging
Child configurations can override specific nested properties while keeping others from the parent:

```yaml
# Parent
agent:
  model_settings:
    model: "gpt-4"
    temperature: 0.7

# Child
agent:
  model_settings:
    temperature: 0.3  # Override only temperature
    # model stays "gpt-4" from parent
```

### Tool Configuration
Each strategy can specify different tool usage patterns:

```yaml
tools:
  - name: "search"
    enabled: true
    config:
      max_results: 5
  - name: "scrape"
    enabled: true
    config:
      timeout: 30
```

## Best Practices

1. **Use Base Configurations**: Create base configs for common settings
2. **Strategy-Specific Overrides**: Only override what's different in child configs
3. **Environment Variables**: Use `{{ENV_VAR}}` for sensitive configuration
4. **Documentation**: Keep README files updated with usage examples
5. **Testing**: Test configurations with different scenarios

## Development

To add a new strategy variant:

1. Create a new YAML file extending the base configuration
2. Override only the strategy-specific settings
3. Add documentation to this README
4. Test with various scraping scenarios

Example:
```yaml
# my-new-strategy.yaml
extends: "base-config.yaml"

agents:
  - name: "scraper-my-strategy"
    strategy:
      type: "my_strategy"
      config:
        my_parameter: "value"
```