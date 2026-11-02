use std::time::Duration;

pub const MAX_CONNECTIONS: usize = 512;
pub const MAX_PHONES_PER_ROOM: usize = 4;
pub const MAX_CONNECTIONS_PER_IP: usize = 60;
pub const IP_WINDOW: Duration = Duration::from_secs(60);
pub const MAX_BYTES_PER_ROOM_PER_DAY: u64 = 1024 * 1024 * 1024;
pub const DAY: Duration = Duration::from_secs(24 * 60 * 60);
pub use hide_relay_protocol::frame::MAX_FRAME;
pub const MAX_IP_RECORDS: usize = 4096;
pub const MAX_ROOM_RECORDS: usize = 4096;
pub const MAX_PENDING_HTTP: usize = 32;
pub const MAX_QUEUED_FRAMES: usize = 4;
/// Discard at most one bounded frame plus the largest WebSocket header on refusal.
pub const MAX_CLOSE_DRAIN_BYTES: usize = MAX_FRAME + 14;
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(6);

/// An embedding may lower the production budgets, never remove or raise them.
#[derive(Clone, Copy)]
pub struct Limits {
    pub connections: usize,
    pub phones_per_room: usize,
    pub connections_per_ip: usize,
    pub bytes_per_room_per_day: u64,
    pub ip_records: usize,
    pub room_records: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            connections: MAX_CONNECTIONS,
            phones_per_room: MAX_PHONES_PER_ROOM,
            connections_per_ip: MAX_CONNECTIONS_PER_IP,
            bytes_per_room_per_day: MAX_BYTES_PER_ROOM_PER_DAY,
            ip_records: MAX_IP_RECORDS,
            room_records: MAX_ROOM_RECORDS,
        }
    }
}
impl Limits {
    pub(crate) fn valid(self) -> bool {
        self.connections > 0
            && self.connections <= MAX_CONNECTIONS
            && self.phones_per_room > 0
            && self.phones_per_room <= MAX_PHONES_PER_ROOM
            && self.connections_per_ip > 0
            && self.connections_per_ip <= MAX_CONNECTIONS_PER_IP
            && self.bytes_per_room_per_day > 0
            && self.bytes_per_room_per_day <= MAX_BYTES_PER_ROOM_PER_DAY
            && self.ip_records > 0
            && self.ip_records <= MAX_IP_RECORDS
            && self.room_records > 0
            && self.room_records <= MAX_ROOM_RECORDS
    }
}
