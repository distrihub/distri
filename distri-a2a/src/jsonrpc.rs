//! JSON-RPC 2.0 envelope, A2A error codes and method names.

use serde::{Deserialize, Serialize};

/// A JSON-RPC request.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
}

impl JsonRpcRequest {
    pub fn new(
        method: impl Into<String>,
        params: serde_json::Value,
        id: Option<serde_json::Value>,
    ) -> Self {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params,
            id,
        }
    }
}

/// A JSON-RPC response.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
}

/// A JSON-RPC error.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Typed read-side view of a JSON-RPC response with result `T`.
#[derive(Deserialize, Debug)]
#[serde(bound(deserialize = "T: serde::Deserialize<'de>"))]
pub struct JsonRpcResponseFor<T> {
    #[serde(default)]
    pub result: Option<T>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<serde_json::Value>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A2A 1.0 error codes (JSON-RPC binding).
pub mod error_code {
    pub const TASK_NOT_FOUND: i32 = -32001;
    pub const TASK_NOT_CANCELABLE: i32 = -32002;
    pub const PUSH_NOTIFICATION_NOT_SUPPORTED: i32 = -32003;
    pub const UNSUPPORTED_OPERATION: i32 = -32004;
    pub const CONTENT_TYPE_NOT_SUPPORTED: i32 = -32005;
    pub const INVALID_AGENT_RESPONSE: i32 = -32006;
    pub const EXTENDED_CARD_NOT_CONFIGURED: i32 = -32007;
    pub const EXTENSION_SUPPORT_REQUIRED: i32 = -32008;
    pub const VERSION_NOT_SUPPORTED: i32 = -32009;
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
}

impl JsonRpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self::new(error_code::INVALID_PARAMS, msg)
    }
    pub fn method_not_found(method: &str) -> Self {
        Self::new(
            error_code::METHOD_NOT_FOUND,
            format!("Method not found: {method}"),
        )
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(error_code::INTERNAL_ERROR, msg)
    }
    pub fn task_not_found(msg: impl Into<String>) -> Self {
        Self::new(error_code::TASK_NOT_FOUND, msg)
    }
    pub fn task_not_cancelable(msg: impl Into<String>) -> Self {
        Self::new(error_code::TASK_NOT_CANCELABLE, msg)
    }
    pub fn push_notification_not_supported() -> Self {
        Self::new(
            error_code::PUSH_NOTIFICATION_NOT_SUPPORTED,
            "Push notifications are not supported",
        )
    }
    pub fn unsupported_operation(msg: impl Into<String>) -> Self {
        Self::new(error_code::UNSUPPORTED_OPERATION, msg)
    }
    pub fn extended_card_not_configured() -> Self {
        Self::new(
            error_code::EXTENDED_CARD_NOT_CONFIGURED,
            "Extended agent card is not configured",
        )
    }
    /// `-32008`: the card marks `uri` required and the request did not
    /// activate it.
    pub fn extension_support_required(uri: &str) -> Self {
        Self {
            code: error_code::EXTENSION_SUPPORT_REQUIRED,
            message: format!("Extension support required: {uri}"),
            data: Some(serde_json::json!({ "uri": uri })),
        }
    }
    pub fn version_not_supported(version: &str) -> Self {
        Self::new(
            error_code::VERSION_NOT_SUPPORTED,
            format!("A2A version not supported: {version}"),
        )
    }
}

/// A2A methods, by their 1.0 names, with the 0.x names they replace.
pub mod methods {
    pub const SEND_MESSAGE: &str = "SendMessage";
    pub const SEND_STREAMING_MESSAGE: &str = "SendStreamingMessage";
    pub const GET_TASK: &str = "GetTask";
    pub const LIST_TASKS: &str = "ListTasks";
    pub const CANCEL_TASK: &str = "CancelTask";
    pub const SUBSCRIBE_TO_TASK: &str = "SubscribeToTask";
    pub const CREATE_PUSH_CONFIG: &str = "CreateTaskPushNotificationConfig";
    pub const GET_PUSH_CONFIG: &str = "GetTaskPushNotificationConfig";
    pub const LIST_PUSH_CONFIGS: &str = "ListTaskPushNotificationConfigs";
    pub const DELETE_PUSH_CONFIG: &str = "DeleteTaskPushNotificationConfig";
    pub const GET_EXTENDED_AGENT_CARD: &str = "GetExtendedAgentCard";

    /// A method, independent of the name it was called by.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Method {
        SendMessage,
        SendStreamingMessage,
        GetTask,
        ListTasks,
        CancelTask,
        SubscribeToTask,
        CreatePushConfig,
        GetPushConfig,
        ListPushConfigs,
        DeletePushConfig,
        GetExtendedAgentCard,
    }

    /// Which spelling the caller used. A 0.x name gets 0.x-shaped results
    /// (see [`crate::legacy`]) for one release.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Dialect {
        V1,
        Legacy,
    }

    impl Method {
        pub fn name(self) -> &'static str {
            match self {
                Method::SendMessage => SEND_MESSAGE,
                Method::SendStreamingMessage => SEND_STREAMING_MESSAGE,
                Method::GetTask => GET_TASK,
                Method::ListTasks => LIST_TASKS,
                Method::CancelTask => CANCEL_TASK,
                Method::SubscribeToTask => SUBSCRIBE_TO_TASK,
                Method::CreatePushConfig => CREATE_PUSH_CONFIG,
                Method::GetPushConfig => GET_PUSH_CONFIG,
                Method::ListPushConfigs => LIST_PUSH_CONFIGS,
                Method::DeletePushConfig => DELETE_PUSH_CONFIG,
                Method::GetExtendedAgentCard => GET_EXTENDED_AGENT_CARD,
            }
        }

        pub fn is_streaming(self) -> bool {
            matches!(self, Method::SendStreamingMessage | Method::SubscribeToTask)
        }

        /// Resolve a 1.0 or 0.x method name.
        pub fn parse(name: &str) -> Option<(Method, Dialect)> {
            use Dialect::*;
            Some(match name {
                SEND_MESSAGE => (Method::SendMessage, V1),
                SEND_STREAMING_MESSAGE => (Method::SendStreamingMessage, V1),
                GET_TASK => (Method::GetTask, V1),
                LIST_TASKS => (Method::ListTasks, V1),
                CANCEL_TASK => (Method::CancelTask, V1),
                SUBSCRIBE_TO_TASK => (Method::SubscribeToTask, V1),
                CREATE_PUSH_CONFIG => (Method::CreatePushConfig, V1),
                GET_PUSH_CONFIG => (Method::GetPushConfig, V1),
                LIST_PUSH_CONFIGS => (Method::ListPushConfigs, V1),
                DELETE_PUSH_CONFIG => (Method::DeletePushConfig, V1),
                GET_EXTENDED_AGENT_CARD => (Method::GetExtendedAgentCard, V1),
                "message/send" => (Method::SendMessage, Legacy),
                "message/stream" => (Method::SendStreamingMessage, Legacy),
                "tasks/get" => (Method::GetTask, Legacy),
                "tasks/list" => (Method::ListTasks, Legacy),
                "tasks/cancel" => (Method::CancelTask, Legacy),
                "tasks/resubscribe" => (Method::SubscribeToTask, Legacy),
                "tasks/pushNotificationConfig/set" => (Method::CreatePushConfig, Legacy),
                "tasks/pushNotificationConfig/get" => (Method::GetPushConfig, Legacy),
                "tasks/pushNotificationConfig/list" => (Method::ListPushConfigs, Legacy),
                "tasks/pushNotificationConfig/delete" => (Method::DeletePushConfig, Legacy),
                "agent/getAuthenticatedExtendedCard" | "agent/authenticatedExtendedCard" => {
                    (Method::GetExtendedAgentCard, Legacy)
                }
                _ => return None,
            })
        }
    }

    pub fn is_streaming(method: &str) -> bool {
        Method::parse(method).is_some_and(|(m, _)| m.is_streaming())
    }
}
