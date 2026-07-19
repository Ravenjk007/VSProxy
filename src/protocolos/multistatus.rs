use std::sync::atomic::{AtomicUsize, Ordering};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Stats {
    pub active_connections: AtomicUsize,
    pub total_websocket: AtomicUsize,
    pub total_socks5: AtomicUsize,
    pub total_security: AtomicUsize,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            active_connections: AtomicUsize::new(0),
            total_websocket: AtomicUsize::new(0),
            total_socks5: AtomicUsize::new(0),
            total_security: AtomicUsize::new(0),
        }
    }

    pub fn add_connection(&self) {
        self.active_connections.fetch_add(1, Ordering::SeqCst);
    }

    pub fn remove_connection(&self) {
        self.active_connections.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn inc_websocket(&self) {
        self.total_websocket.fetch_add(1, Ordering::SeqCst);
    }

    pub fn inc_socks5(&self) {
        self.total_socks5.fetch_add(1, Ordering::SeqCst);
    }

    pub fn inc_security(&self) {
        self.total_security.fetch_add(1, Ordering::SeqCst);
    }
}
