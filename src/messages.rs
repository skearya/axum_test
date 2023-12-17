use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientMsg {
    Username { username: String },
    ChatMessage { content: String },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type", content = "data")]
pub enum ServerMsg {
    UsernameTaken,
    UserConnected { username: String },
    UserDisconnected { username: String },
    ChatMessage { username: String, content: String },
}

#[derive(Clone, Debug)]
pub enum ServerToServer {
    UserConnected { username: String },
    UserDisconnected { username: String },
    ChatMessage { username: String, content: String },
}
