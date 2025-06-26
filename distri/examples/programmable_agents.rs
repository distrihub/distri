use distri::{
    Agent, AgentContext, AgentDefinition, AgentResponse, ModelSettings, 
    ProgrammableAgent, TaskStep, Artifact
};
use distri::error::AgentError;
use distri::coordinator::{LocalCoordinator, CoordinatorContext, AgentCoordinator};
use distri::servers::registry::ServerRegistry;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Example 1: Simple Text Processing Agent
struct TextProcessorAgent {
    definition: AgentDefinition,
}

impl TextProcessorAgent {
    fn new() -> Self {
        let definition = AgentDefinition {
            name: "text_processor".to_string(),
            description: "An agent that processes text in various ways".to_string(),
            system_prompt: Some("You are a text processing agent. You can transform text in various ways.".to_string()),
            mcp_servers: vec![],
            model_settings: ModelSettings::default(),
            parameters: None,
            response_format: None,
            history_size: None,
            plan: None,
            icon_url: Some("https://example.com/text-processor.png".to_string()),
        };

        Self { definition }
    }
}

#[async_trait::async_trait]
impl Agent for TextProcessorAgent {
    fn definition(&self) -> &AgentDefinition {
        &self.definition
    }

    async fn invoke(
        &mut self,
        task: TaskStep,
        _context: AgentContext,
        _params: Option<serde_json::Value>,
    ) -> Result<AgentResponse, AgentError> {
        let text = &task.task;
        
        // Simple text processing logic
        let processed = if text.contains("uppercase") {
            text.to_uppercase()
        } else if text.contains("lowercase") {
            text.to_lowercase()
        } else if text.contains("reverse") {
            text.chars().rev().collect()
        } else if text.contains("word_count") {
            format!("Word count: {}", text.split_whitespace().count())
        } else {
            format!("Processed: {}", text)
        };

        Ok(AgentResponse::text(processed))
    }
}

/// Example 2: Math Calculator Agent using Builder Pattern
fn create_calculator_agent() -> ProgrammableAgent {
    ProgrammableAgent::builder("calculator")
        .description("A mathematical calculator agent")
        .system_prompt("You are a calculator. You can perform basic mathematical operations.")
        .icon_url("https://example.com/calculator.png")
        .handler(|task, _context| async move {
            let expression = &task.task;
            
            // Simple expression evaluator (in real implementation, use a proper parser)
            let result = if expression.contains('+') {
                let parts: Vec<&str> = expression.split('+').collect();
                if parts.len() == 2 {
                    let a: f64 = parts[0].trim().parse().unwrap_or(0.0);
                    let b: f64 = parts[1].trim().parse().unwrap_or(0.0);
                    format!("{} + {} = {}", a, b, a + b)
                } else {
                    "Invalid addition expression".to_string()
                }
            } else if expression.contains('-') {
                let parts: Vec<&str> = expression.split('-').collect();
                if parts.len() == 2 {
                    let a: f64 = parts[0].trim().parse().unwrap_or(0.0);
                    let b: f64 = parts[1].trim().parse().unwrap_or(0.0);
                    format!("{} - {} = {}", a, b, a - b)
                } else {
                    "Invalid subtraction expression".to_string()
                }
            } else if expression.contains('*') {
                let parts: Vec<&str> = expression.split('*').collect();
                if parts.len() == 2 {
                    let a: f64 = parts[0].trim().parse().unwrap_or(0.0);
                    let b: f64 = parts[1].trim().parse().unwrap_or(0.0);
                    format!("{} * {} = {}", a, b, a * b)
                } else {
                    "Invalid multiplication expression".to_string()
                }
            } else {
                format!("I can calculate: {}", expression)
            };

            Ok(AgentResponse::text(result))
        })
        .build()
}

/// Example 3: Data Analysis Agent with Artifacts
struct DataAnalysisAgent {
    definition: AgentDefinition,
}

impl DataAnalysisAgent {
    fn new() -> Self {
        let definition = AgentDefinition {
            name: "data_analyst".to_string(),
            description: "An agent that performs data analysis and generates reports".to_string(),
            system_prompt: Some("You are a data analyst. You analyze data and generate comprehensive reports.".to_string()),
            mcp_servers: vec![],
            model_settings: ModelSettings::default(),
            parameters: None,
            response_format: None,
            history_size: None,
            plan: None,
            icon_url: Some("https://example.com/data-analyst.png".to_string()),
        };

        Self { definition }
    }
}

#[async_trait::async_trait]
impl Agent for DataAnalysisAgent {
    fn definition(&self) -> &AgentDefinition {
        &self.definition
    }

    async fn invoke(
        &mut self,
        task: TaskStep,
        context: AgentContext,
        _params: Option<serde_json::Value>,
    ) -> Result<AgentResponse, AgentError> {
        let data_request = &task.task;
        
        // Simulate data analysis
        let analysis_result = format!("Data analysis for: {}", data_request);
        
        // Create artifacts
        let csv_data = "name,value\nSample A,100\nSample B,200\nSample C,150";
        let json_report = serde_json::json!({
            "summary": "Data analysis complete",
            "total_samples": 3,
            "average_value": 150,
            "thread_id": context.thread_id
        });

        let artifacts = vec![
            Artifact {
                id: uuid::Uuid::new_v4().to_string(),
                name: "data.csv".to_string(),
                content_type: "text/csv".to_string(),
                content: csv_data.to_string(),
                metadata: Some(serde_json::json!({"rows": 3})),
            },
            Artifact {
                id: uuid::Uuid::new_v4().to_string(),
                name: "report.json".to_string(),
                content_type: "application/json".to_string(),
                content: json_report.to_string(),
                metadata: Some(serde_json::json!({"generated_at": chrono::Utc::now()})),
            },
        ];

        Ok(AgentResponse::text(analysis_result).with_artifacts(artifacts))
    }
}

/// Example 4: Stateful Agent with Memory
struct CounterAgent {
    definition: AgentDefinition,
    counter: u32,
}

impl CounterAgent {
    fn new() -> Self {
        let definition = AgentDefinition {
            name: "counter".to_string(),
            description: "An agent that maintains a counter state".to_string(),
            system_prompt: Some("You are a counter agent. You can increment, decrement, or reset a counter.".to_string()),
            mcp_servers: vec![],
            model_settings: ModelSettings::default(),
            parameters: None,
            response_format: None,
            history_size: None,
            plan: None,
            icon_url: Some("https://example.com/counter.png".to_string()),
        };

        Self { 
            definition,
            counter: 0,
        }
    }
}

#[async_trait::async_trait]
impl Agent for CounterAgent {
    fn definition(&self) -> &AgentDefinition {
        &self.definition
    }

    async fn invoke(
        &mut self,
        task: TaskStep,
        _context: AgentContext,
        _params: Option<serde_json::Value>,
    ) -> Result<AgentResponse, AgentError> {
        let command = task.task.to_lowercase();
        
        let result = if command.contains("increment") || command.contains("inc") {
            self.counter += 1;
            format!("Counter incremented to: {}", self.counter)
        } else if command.contains("decrement") || command.contains("dec") {
            if self.counter > 0 {
                self.counter -= 1;
            }
            format!("Counter decremented to: {}", self.counter)
        } else if command.contains("reset") {
            self.counter = 0;
            "Counter reset to: 0".to_string()
        } else if command.contains("get") || command.contains("value") {
            format!("Current counter value: {}", self.counter)
        } else {
            format!("Counter is at: {}. Commands: increment, decrement, reset, get", self.counter)
        };

        Ok(AgentResponse::text(result))
    }
}

/// Example usage function
pub async fn run_examples() -> anyhow::Result<()> {
    println!("🤖 Distri Programmable Agents Examples");
    
    // Initialize coordinator
    let registry = Arc::new(RwLock::new(ServerRegistry::new()));
    let context = Arc::new(CoordinatorContext::default());
    let coordinator = Arc::new(LocalCoordinator::new(
        registry,
        None,
        None,
        context,
    ));

    // Example 1: Register and use TextProcessorAgent
    println!("\n📝 Example 1: Text Processor Agent");
    let text_processor = Box::new(TextProcessorAgent::new());
    coordinator.register_programmable_agent(text_processor).await?;

    let result = coordinator.execute(
        "text_processor",
        TaskStep {
            task: "Please uppercase this text".to_string(),
            task_images: None,
        },
        None,
    ).await?;
    println!("Result: {}", result);

    // Example 2: Register and use Calculator Agent (built with builder pattern)
    println!("\n🔢 Example 2: Calculator Agent");
    let calculator = Box::new(create_calculator_agent());
    coordinator.register_programmable_agent(calculator).await?;

    let result = coordinator.execute(
        "calculator",
        TaskStep {
            task: "10 + 5".to_string(),
            task_images: None,
        },
        None,
    ).await?;
    println!("Result: {}", result);

    // Example 3: Register and use Data Analysis Agent
    println!("\n📊 Example 3: Data Analysis Agent");
    let data_analyst = Box::new(DataAnalysisAgent::new());
    coordinator.register_programmable_agent(data_analyst).await?;

    let result = coordinator.execute(
        "data_analyst",
        TaskStep {
            task: "Analyze sales data for Q4".to_string(),
            task_images: None,
        },
        None,
    ).await?;
    println!("Result: {}", result);

    // Example 4: Register and use Stateful Counter Agent
    println!("\n🔢 Example 4: Stateful Counter Agent");
    let counter = Box::new(CounterAgent::new());
    coordinator.register_programmable_agent(counter).await?;

    // Multiple operations on the same agent
    for command in &["increment", "increment", "get", "decrement", "reset", "value"] {
        let result = coordinator.execute(
            "counter",
            TaskStep {
                task: command.to_string(),
                task_images: None,
            },
            None,
        ).await?;
        println!("Command '{}': {}", command, result);
    }

    // List all agents
    println!("\n📋 All Registered Agents:");
    let (agents, _) = coordinator.list_agents(None).await?;
    for agent in agents {
        println!("- {}: {}", agent.name, agent.description);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    run_examples().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_text_processor_agent() {
        let mut agent = TextProcessorAgent::new();
        let context = AgentContext::default();
        
        let task = TaskStep {
            task: "uppercase this text".to_string(),
            task_images: None,
        };
        
        let result = agent.invoke(task, context, None).await.unwrap();
        assert!(result.content.contains("UPPERCASE"));
    }

    #[tokio::test]
    async fn test_calculator_agent() {
        let mut agent = create_calculator_agent();
        let context = AgentContext::default();
        
        let task = TaskStep {
            task: "5 + 3".to_string(),
            task_images: None,
        };
        
        let result = agent.invoke(task, context, None).await.unwrap();
        assert!(result.content.contains("8"));
    }

    #[tokio::test]
    async fn test_counter_agent() {
        let mut agent = CounterAgent::new();
        let context = AgentContext::default();
        
        // Test increment
        let task = TaskStep {
            task: "increment".to_string(),
            task_images: None,
        };
        
        let result = agent.invoke(task, context.clone(), None).await.unwrap();
        assert!(result.content.contains("1"));
        
        // Test get value
        let task = TaskStep {
            task: "get".to_string(),
            task_images: None,
        };
        
        let result = agent.invoke(task, context, None).await.unwrap();
        assert!(result.content.contains("1"));
    }
}