//! In-process test adapters; re-exported to preserve existing callers.
use super::{IpcMessage, SessionTable};

/// In-process fake broker for tests (no OS pipe).
pub struct FakeBroker {
    pub table: SessionTable,
    pub seen: Vec<IpcMessage>,
}

impl FakeBroker {
    pub fn new() -> Self {
        Self {
            table: SessionTable::new(),
            seen: Vec::new(),
        }
    }

    pub fn send(&mut self, msg: IpcMessage) -> Option<IpcMessage> {
        self.seen.push(msg.clone());
        self.table.handle(&msg)
    }
}

impl Default for FakeBroker {
    fn default() -> Self {
        Self::new()
    }
}

/// Round-trip helper used by unit tests.
pub fn round_trip(msg: &IpcMessage) -> IpcMessage {
    IpcMessage::decode_line(&msg.encode_line()).expect("protocol round-trip")
}
