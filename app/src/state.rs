use std::collections::HashMap;
use std::sync::Arc;

use socketioxide::extract::SocketRef;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::session::Session;

#[derive(Debug)]
pub struct AppState {
    pub sessions: RwLock<HashMap<String, Session>>,
    pub current_session: RwLock<Option<Arc<str>>>,
    pub sockets: RwLock<HashMap<String, SocketRef>>,
    pub websocket_ice_queue: RwLock<HashMap<String, Vec<String>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            current_session: RwLock::new(None),
            sockets: RwLock::new(HashMap::new()),
            websocket_ice_queue: RwLock::new(HashMap::new()),
        }
    }

    pub async fn add_session(&self, session: Session) {
        let session_id = session.id.clone();
        self.sessions.write().await.insert(session_id.to_string(), session);

        let mut current = self.current_session.write().await;
        if current.is_none() {
            *current = Some(session_id);
        }
    }

    pub async fn remove_session(&self, session_id: &str) -> Option<Session> {
        let removed = self.sessions.write().await.remove(session_id);

        let mut current = self.current_session.write().await;
        if let Some(ref current_id) = *current
            && current_id.as_ref() == session_id
        {
            *current = None;
        }

        removed
    }

    pub async fn get_current_session(&self) -> Option<Arc<str>> {
        self.current_session.read().await.clone()
    }

    pub async fn set_current_session(&self, session_id: Option<Arc<str>>) {
        *self.current_session.write().await = session_id;
    }

    pub async fn get_session(&self, session_id: &str) -> Option<Session> {
        self.sessions.read().await.get(session_id).cloned()
    }

    pub async fn queue_ice_candidate(&self, session_id: &str, candidate: String) {
        const MAX_ICE_CANDIDATES: usize = 20;

        let mut queue = self.websocket_ice_queue.write().await;
        let ice_queue = queue.entry(session_id.to_string()).or_default();

        if ice_queue.len() >= MAX_ICE_CANDIDATES {
            ice_queue.remove(0);
            warn!("ICE queue full for session {}, removed oldest candidate", session_id);
        }

        ice_queue.push(candidate);
    }

    pub async fn get_ice_candidates(&self, session_id: &str) -> Vec<String> {
        let candidates =
            self.websocket_ice_queue.write().await.remove(session_id).unwrap_or_default();
        if !candidates.is_empty() {
            debug!("Retrieved {} ICE candidates for session {}", candidates.len(), session_id);
        }
        candidates
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
