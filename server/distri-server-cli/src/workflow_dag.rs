use anyhow::Result;
use deno_ast::{parse_module, MediaType, ParseParams};
use distri_types::workflow::{WorkflowDAG, WorkflowEdge, WorkflowNode, WorkflowNodeType};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

pub struct WorkflowDAGGenerator {
    node_counter: u32,
}

impl WorkflowDAGGenerator {
    pub fn new() -> Self {
        Self { node_counter: 0 }
    }

    pub fn generate_dag_from_file(&mut self, file_path: &Path) -> Result<WorkflowDAG> {
        let content = std::fs::read_to_string(file_path)?;
        self.generate_dag_from_content(&content)
    }

    pub fn generate_dag_from_content(&mut self, content: &str) -> Result<WorkflowDAG> {
        // Try to parse with deno_ast for validation
        let specifier_url = "workflow.ts"
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse specifier URL: {}", e))?;

        let _parsed = parse_module(ParseParams {
            specifier: specifier_url,
            text: content.into(),
            media_type: MediaType::TypeScript,
            capture_tokens: false,
            scope_analysis: false,
            maybe_syntax: None,
        })?;

        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        // Add start node
        let start_node_id = self.next_node_id();
        nodes.push(WorkflowNode {
            id: start_node_id.clone(),
            node_type: WorkflowNodeType::Start,
            label: "Start".to_string(),
            position: Some((0.0, 0.0)),
            metadata: HashMap::new(),
        });

        // For now, do a simple text-based analysis as fallback
        // since the full AST walking is complex with the current deno_ast version
        let current_node =
            self.analyze_text_content(content, &mut nodes, &mut edges, start_node_id)?;

        // Add end node
        let end_node_id = self.next_node_id();
        nodes.push(WorkflowNode {
            id: end_node_id.clone(),
            node_type: WorkflowNodeType::End,
            label: "End".to_string(),
            position: None,
            metadata: HashMap::new(),
        });

        // Connect last node to end
        edges.push(WorkflowEdge {
            id: format!("edge_{}", self.node_counter),
            source: current_node,
            target: end_node_id,
            label: None,
            condition: None,
        });

        Ok(WorkflowDAG {
            nodes,
            edges,
            layout: Some("hierarchical".to_string()),
        })
    }

    fn analyze_text_content(
        &mut self,
        content: &str,
        nodes: &mut Vec<WorkflowNode>,
        edges: &mut Vec<WorkflowEdge>,
        mut current_node: String,
    ) -> Result<String> {
        let lines: Vec<&str> = content.lines().collect();

        for line in lines {
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
                continue;
            }

            // Check for function calls we care about
            if let Some(node_id) = self.analyze_line(trimmed, nodes, edges, &current_node)? {
                current_node = node_id;
            }
        }

        Ok(current_node)
    }

    fn analyze_line(
        &mut self,
        line: &str,
        nodes: &mut Vec<WorkflowNode>,
        edges: &mut Vec<WorkflowEdge>,
        current_node: &str,
    ) -> Result<Option<String>> {
        // Check for important function calls
        if let Some(function_name) = self.extract_function_call(line) {
            if self.is_important_function(&function_name) {
                let node_id = self.next_node_id();
                let node_type = self.classify_function_call(&function_name);

                nodes.push(WorkflowNode {
                    id: node_id.clone(),
                    node_type,
                    label: function_name.clone(),
                    position: None,
                    metadata: {
                        let mut meta = HashMap::new();
                        meta.insert("function_name".to_string(), Value::String(function_name));
                        meta.insert("source_line".to_string(), Value::String(line.to_string()));
                        meta
                    },
                });

                edges.push(WorkflowEdge {
                    id: format!("edge_{}", self.node_counter),
                    source: current_node.to_string(),
                    target: node_id.clone(),
                    label: None,
                    condition: None,
                });

                return Ok(Some(node_id));
            }
        }

        // Check for control flow structures
        if line.contains("if") && line.contains("(") {
            let condition = self.extract_condition(line);
            let node_id = self.next_node_id();

            nodes.push(WorkflowNode {
                id: node_id.clone(),
                node_type: WorkflowNodeType::Condition {
                    expression: condition,
                },
                label: "Condition".to_string(),
                position: None,
                metadata: {
                    let mut meta = HashMap::new();
                    meta.insert("source_line".to_string(), Value::String(line.to_string()));
                    meta
                },
            });

            edges.push(WorkflowEdge {
                id: format!("edge_{}", self.node_counter),
                source: current_node.to_string(),
                target: node_id.clone(),
                label: None,
                condition: None,
            });

            return Ok(Some(node_id));
        }

        // Check for loops
        if line.contains("for") && line.contains("(") {
            let node_id = self.next_node_id();

            nodes.push(WorkflowNode {
                id: node_id.clone(),
                node_type: WorkflowNodeType::Loop {
                    variable: "i".to_string(),
                    iterable: "loop".to_string(),
                },
                label: "For Loop".to_string(),
                position: None,
                metadata: {
                    let mut meta = HashMap::new();
                    meta.insert("source_line".to_string(), Value::String(line.to_string()));
                    meta
                },
            });

            edges.push(WorkflowEdge {
                id: format!("edge_{}", self.node_counter),
                source: current_node.to_string(),
                target: node_id.clone(),
                label: None,
                condition: None,
            });

            return Ok(Some(node_id));
        }

        if line.contains("while") && line.contains("(") {
            let condition = self.extract_condition(line);
            let node_id = self.next_node_id();

            nodes.push(WorkflowNode {
                id: node_id.clone(),
                node_type: WorkflowNodeType::Loop {
                    variable: "condition".to_string(),
                    iterable: condition,
                },
                label: "While Loop".to_string(),
                position: None,
                metadata: {
                    let mut meta = HashMap::new();
                    meta.insert("source_line".to_string(), Value::String(line.to_string()));
                    meta
                },
            });

            edges.push(WorkflowEdge {
                id: format!("edge_{}", self.node_counter),
                source: current_node.to_string(),
                target: node_id.clone(),
                label: None,
                condition: None,
            });

            return Ok(Some(node_id));
        }

        Ok(None)
    }

    fn extract_function_call(&self, line: &str) -> Option<String> {
        // Simple regex-like parsing for function calls
        if let Some(call_start) = line.find('(') {
            // Look backwards to find the function name
            let before_paren = &line[..call_start];

            // Find the function name (could be obj.method or just function)
            if let Some(dot_pos) = before_paren.rfind('.') {
                // Method call: obj.method()
                let method_part = &before_paren[dot_pos + 1..];
                if let Some(space_pos) = method_part.rfind(' ') {
                    Some(method_part[space_pos + 1..].trim().to_string())
                } else {
                    Some(method_part.trim().to_string())
                }
            } else {
                // Regular function call: function()
                if let Some(space_pos) = before_paren.rfind(' ') {
                    Some(before_paren[space_pos + 1..].trim().to_string())
                } else {
                    // Handle cases like "await function(" or "return function("
                    let words: Vec<&str> = before_paren.split_whitespace().collect();
                    words.last().map(|s| s.to_string())
                }
            }
        } else {
            None
        }
    }

    fn extract_condition(&self, line: &str) -> String {
        if let (Some(start), Some(end)) = (line.find('('), line.rfind(')')) {
            if start < end {
                line[start + 1..end].trim().to_string()
            } else {
                "condition".to_string()
            }
        } else {
            "condition".to_string()
        }
    }

    fn is_important_function(&self, function_name: &str) -> bool {
        matches!(
            function_name,
            "callAgent" | "callTool" | "executeWorkflow" | "runTask" | "processData"
        ) || function_name.starts_with("call")
            || function_name.starts_with("execute")
            || function_name.starts_with("run")
            || function_name.starts_with("process")
    }

    fn classify_function_call(&self, function_name: &str) -> WorkflowNodeType {
        if function_name == "callAgent" || function_name.contains("Agent") {
            WorkflowNodeType::AgentCall {
                function_name: function_name.to_string(),
            }
        } else if function_name == "callTool" || function_name.contains("Tool") {
            WorkflowNodeType::ToolCall {
                tool_name: function_name.to_string(),
            }
        } else {
            WorkflowNodeType::AgentCall {
                function_name: function_name.to_string(),
            }
        }
    }

    fn next_node_id(&mut self) -> String {
        self.node_counter += 1;
        format!("node_{}", self.node_counter)
    }
}

impl Default for WorkflowDAGGenerator {
    fn default() -> Self {
        Self::new()
    }
}
