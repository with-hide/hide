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
pub const MAX_QUEUED_FRAMES: usize = 8;
pub const MAX_QUEUED_CONTROLS: usize = 16;
/// Discard at most one bounded frame plus the largest WebSocket header on refusal.
pub const MAX_CLOSE_DRAIN_BYTES: usize = MAX_FRAME + 14;
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
// HTTP response, upgrade and final close can each consume their deadline.
// This also covers a session write followed by its final close write.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(16);

/// Push relay budgets (`docs/RELAY_PROTOCOL.md`, Push relay). Each room and
/// the whole relay are capped, and every request that reaches the cap check is
/// counted, a refused one included, so a room cannot spend the relay's CPU
/// without a ceiling of its own. One set of counters counts Apple's
/// `push_request` and Google's `fcm_request` together: a room's budget is the
/// same whichever service its phones use.
pub const PUSHES_PER_ROOM_PER_MINUTE: usize = 20;
pub const PUSHES_PER_ROOM_PER_DAY: usize = 1_000;
pub const PUSHES_PER_SECOND: usize = 50;
pub const PUSH_MINUTE: Duration = Duration::from_secs(60);
/// Requests in flight to Apple and Google together, each a stream of one of
/// the two HTTP/2 connections.
pub const MAX_CONCURRENT_PUSHES: usize = 32;
/// One request to Apple or Google, its answer, its body and its one retry,
/// an access token fetch included.
pub const PUSH_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// One shared provider token, replaced at this age: inside Apple's hour, and
/// past its 20-minute floor (TooManyProviderTokenUpdates).
pub const PROVIDER_TOKEN_REFRESH: Duration = Duration::from_secs(50 * 60);
/// The youngest token the relay replaces when Apple calls it expired; a
/// younger one is answered `unavailable` instead of re-signed.
pub const PROVIDER_TOKEN_MIN_AGE: Duration = Duration::from_secs(20 * 60);
/// Google's access token is fetched again once it has this little left, so a
/// send never carries one that expires on the way.
pub const FCM_ACCESS_TOKEN_MARGIN: Duration = Duration::from_secs(5 * 60);
/// The longest the relay keeps an access token, whatever lifetime the
/// metadata server names; it issues one-hour tokens.
pub const FCM_ACCESS_TOKEN_MAX_LIFETIME: Duration = Duration::from_secs(60 * 60);

/// A push budget set lower than production for a test; an embedding may
/// lower these, never remove or raise them.
#[derive(Clone, Copy)]
pub struct PushLimits {
    pub per_room_per_minute: usize,
    pub per_room_per_day: usize,
    pub per_second: usize,
    pub concurrent: usize,
    pub request_timeout: Duration,
}
impl Default for PushLimits {
    fn default() -> Self {
        Self {
            per_room_per_minute: PUSHES_PER_ROOM_PER_MINUTE,
            per_room_per_day: PUSHES_PER_ROOM_PER_DAY,
            per_second: PUSHES_PER_SECOND,
            concurrent: MAX_CONCURRENT_PUSHES,
            request_timeout: PUSH_REQUEST_TIMEOUT,
        }
    }
}
impl PushLimits {
    pub(crate) fn valid(self) -> bool {
        self.per_room_per_minute > 0
            && self.per_room_per_minute <= PUSHES_PER_ROOM_PER_MINUTE
            && self.per_room_per_day > 0
            && self.per_room_per_day <= PUSHES_PER_ROOM_PER_DAY
            && self.per_second > 0
            && self.per_second <= PUSHES_PER_SECOND
            && self.concurrent > 0
            && self.concurrent <= MAX_CONCURRENT_PUSHES
            && !self.request_timeout.is_zero()
            && self.request_timeout <= PUSH_REQUEST_TIMEOUT
    }
}

/// An embedding may lower the production budgets, never remove or raise them.
#[derive(Clone, Copy)]
pub struct Limits {
    pub idle_timeout: Duration,
    pub queued_controls: usize,
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
            idle_timeout: IDLE_TIMEOUT,
            queued_controls: MAX_QUEUED_CONTROLS,
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
        !self.idle_timeout.is_zero()
            && self.idle_timeout <= IDLE_TIMEOUT
            && self.queued_controls > 0
            && self.queued_controls <= MAX_QUEUED_CONTROLS
            && self.connections > 0
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
