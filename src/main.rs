mod messages;

use axum::{
    async_trait,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::header,
    response::IntoResponse,
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use futures::{stream::SplitSink, SinkExt, StreamExt};
use messages::{ClientMsg, ServerMsg, ServerToServer};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::{
    fs,
    sync::{broadcast, mpsc},
};

#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<ServerToServer>,
    users: Arc<Mutex<HashSet<String>>>,
    starting_time: DateTime<Utc>,
}

#[tokio::main]
async fn main() {
    let (tx, _) = broadcast::channel::<ServerToServer>(16);

    let state = AppState {
        tx,
        users: Arc::new(Mutex::new(HashSet::new())),
        starting_time: Utc::now(),
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/ws", get(websocket_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn root() -> impl IntoResponse {
    let html = fs::read_to_string("src/index.html").await.unwrap();

    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html)
}

async fn websocket_handler(ws: WebSocketUpgrade, state: State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| websocket(socket, state))
}

async fn websocket(stream: WebSocket, state: State<AppState>) {
    let (mut client_tx, mut client_rx) = stream.split();

    let mut username = String::new();

    while let Some(Ok(Message::Text(message))) = client_rx.next().await {
        let message: ClientMsg = serde_json::from_str(&message).unwrap();

        if let ClientMsg::Username { username: desired } = message {
            if desired != "" && state.users.lock().unwrap().insert(desired.clone()) {
                username = desired;

                let _ = state.tx.send(ServerToServer::UserConnected {
                    username: username.clone(),
                });

                break;
            } else {
                client_tx.send_serialized(ServerMsg::UsernameTaken).await;
            }
        }
    }

    let (client_proxy_tx, mut client_proxy_rx) = mpsc::channel::<ServerMsg>(32);

    let client_proxy_task = tokio::spawn(async move {
        while let Some(message) = client_proxy_rx.recv().await {
            let _ = client_tx.send_serialized(message).await;
        }
    });

    // this channel exists because i want to use client_tx in 2 different tasks and i cant clone or use a arc+mutex on client_tx

    let mut client_task = {
        let username = username.clone();
        let server_tx = state.tx.clone();
        let client_proxy_tx = client_proxy_tx.clone();
        let starting_time = state.starting_time.clone();

        tokio::spawn(async move {
            while let Some(Ok(Message::Text(message))) = client_rx.next().await {
                let message: ClientMsg = serde_json::from_str(&message).unwrap();

                if let ClientMsg::ChatMessage { content } = message {
                    if content.starts_with("/uptime") {
                        let time = Utc::now() - starting_time;

                        let _ = client_proxy_tx
                            .send(ServerMsg::ChatMessage {
                                username: "Server".into(),
                                content: format!("{} seconds", time.num_seconds()),
                            })
                            .await;
                    } else {
                        let _ = server_tx.send(ServerToServer::ChatMessage {
                            username: username.clone(),
                            content,
                        });
                    }
                }
            }
        })
    };

    let mut broadcast_task = {
        let mut server_rx = state.tx.clone().subscribe();

        tokio::spawn(async move {
            while let Ok(message) = server_rx.recv().await {
                match message {
                    ServerToServer::ChatMessage { username, content } => {
                        let _ = client_proxy_tx
                            .send(ServerMsg::ChatMessage { username, content })
                            .await;
                    }
                    ServerToServer::UserConnected { username } => {
                        let _ = client_proxy_tx
                            .send(ServerMsg::UserConnected { username })
                            .await;
                    }
                    ServerToServer::UserDisconnected { username } => {
                        let _ = client_proxy_tx
                            .send(ServerMsg::UserDisconnected { username })
                            .await;
                    }
                }
            }
        })
    };

    tokio::select! {
        _ = (&mut client_task) => broadcast_task.abort(),
        _ = (&mut broadcast_task) => client_task.abort()
    }

    client_proxy_task.abort();

    if state.users.lock().unwrap().remove(&username) {
        let _ = state
            .tx
            .clone()
            .send(ServerToServer::UserDisconnected { username });
    }
}

#[async_trait]
trait ServerSend {
    async fn send_serialized(&mut self, message: ServerMsg);
}

#[async_trait]
impl ServerSend for SplitSink<WebSocket, Message> {
    async fn send_serialized(&mut self, message: ServerMsg) {
        let serialized = serde_json::to_string(&message).unwrap();
        let _ = self.send(serialized.into()).await;
    }
}
