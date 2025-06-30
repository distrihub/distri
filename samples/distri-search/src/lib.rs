//! # DeepSearch Agent Examples
//!
//! This crate contains two examples demonstrating different approaches to building 
//! research agents with the distri framework:
//!
//! ## Examples
//!
//! ### 1. YAML-based Standard Agent (`yaml_agent_example`)
//! - Uses the built-in distri Agent (Agent::new_local)  
//! - Configuration-driven via YAML file
//! - Leverages LLM reasoning for tool orchestration
//! - Simple to configure and deploy
//!
//! ### 2. Custom Agent Implementation (`custom_agent_example`)  
//! - Implements the CustomAgent trait
//! - Explicit workflow logic in Rust code
//! - Full control over multi-step orchestration
//! - Programmatic tool call generation
//!
//! ## Running the Examples
//!
//! ```bash
//! # YAML-based agent example
//! cargo run --bin yaml_agent_example
//!
//! # Custom agent example  
//! cargo run --bin custom_agent_example
//! ```
//!
//! ## Key Differences
//!
//! | Approach | Tool Orchestration | Configuration | Use Case |
//! |----------|-------------------|---------------|----------|
//! | YAML Agent | LLM-driven | YAML file | Quick prototypes, flexible reasoning |
//! | Custom Agent | Code-driven | Rust implementation | Complex workflows, deterministic logic |

#![doc(html_root_url = "https://docs.rs/distri-search/")]