use crate::{event::Event, limits::*};
use hide_relay_protocol::{
    admission::{DeviceHello, Refusal, ServerMessage},
    frame::Frame,
};
use std::{collections::HashMap, net::IpAddr, sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Rejection {
    pub reason: Refusal,
    pub event: &'static str,
    pub cap: Option<&'static str>,
}
impl Rejection {
    pub fn new(reason: Refusal, event: &'static str) -> Self {
        Self {
            reason,
            event,
            cap: None,
        }
    }
    pub fn cap(reason: Refusal, cap: &'static str) -> Self {
        Self {
            reason,
            event: "cap_exceeded",
            cap: Some(cap),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum End {
    Refused(Rejection),
    PeerClosed,
    Restart,
}

#[derive(Clone)]
pub(crate) enum Role {
    Pending,
    Device(String),
    Phone(String, u32),
}
impl Role {
    fn room(&self) -> Option<&str> {
        match self {
            Self::Pending => None,
            Self::Device(room) | Self::Phone(room, _) => Some(room),
        }
    }
}
pub(crate) struct Outbound {
    pub message: Message,
    // Separate budgets reserve lifecycle capacity while one FIFO preserves
    // PeerJoined-before-data and data-before-PeerLeft ordering.
    pub capacity: OwnedSemaphorePermit,
}
struct Peer {
    role: Role,
    output: mpsc::Sender<Outbound>,
    data_capacity: Arc<Semaphore>,
    control_capacity: Arc<Semaphore>,
    departure_capacity: Option<OwnedSemaphorePermit>,
    end: watch::Sender<Option<End>>,
    bytes: u64,
}
struct Room {
    device: u64,
    protocol: u32,
    next_connection: u32,
    phones: HashMap<u32, u64>,
}
impl Room {
    fn phone(&self, connection: u32) -> Result<Option<u64>, Rejection> {
        if connection == 0 || connection > self.next_connection {
            return Err(invalid());
        }
        Ok(self.phones.get(&connection).copied())
    }
}
struct Usage {
    started: Duration,
    bytes: u64,
}
struct Rate {
    started: Duration,
    count: usize,
}

/// A room's push spending, for the caps `docs/RELAY_PROTOCOL.md` documents.
#[derive(Default)]
struct PushUsage {
    minute: Option<Rate>,
    day: Option<Rate>,
}
impl PushUsage {
    /// Counts one request, refused ones included, and answers whether the
    /// room is still inside both caps. `None` means over a cap, with its name.
    fn spend(&mut self, now: Duration, limits: PushLimits) -> Result<(), &'static str> {
        let minute = self.minute.get_or_insert(Rate {
            started: now,
            count: 0,
        });
        if now.saturating_sub(minute.started) >= PUSH_MINUTE {
            *minute = Rate {
                started: now,
                count: 0,
            };
        }
        if minute.count >= limits.per_room_per_minute {
            return Err("pushes_per_room_per_minute");
        }
        let day = self.day.get_or_insert(Rate {
            started: now,
            count: 0,
        });
        if now.saturating_sub(day.started) >= DAY {
            *day = Rate {
                started: now,
                count: 0,
            };
        }
        if day.count >= limits.per_room_per_day {
            return Err("pushes_per_room_per_day");
        }
        minute.count += 1;
        day.count += 1;
        Ok(())
    }
}

pub(crate) struct State {
    stopping: bool,
    limits: Limits,
    push: PushLimits,
    /// Whether this relay has Apple's key, and so forwards push requests.
    push_enabled: bool,
    next_socket: u64,
    peers: HashMap<u64, Peer>,
    rooms: HashMap<String, Room>,
    usage: HashMap<String, Usage>,
    rates: HashMap<IpAddr, Rate>,
    /// Every room's push spending, and the relay's own per-second count.
    pushes: HashMap<String, PushUsage>,
    push_second: Option<Rate>,
}
impl State {
    pub fn new(limits: Limits, push: PushLimits, push_enabled: bool) -> Self {
        Self {
            stopping: false,
            limits,
            push,
            push_enabled,
            next_socket: 0,
            peers: HashMap::new(),
            rooms: HashMap::new(),
            usage: HashMap::new(),
            rates: HashMap::new(),
            pushes: HashMap::new(),
            push_second: None,
        }
    }

    pub fn reserve(
        &mut self,
        ip: IpAddr,
        now: Duration,
        output: mpsc::Sender<Outbound>,
        end: watch::Sender<Option<End>>,
    ) -> Result<u64, Rejection> {
        if self.stopping {
            return Err(Rejection::new(Refusal::Timeout, "server_stopping"));
        }
        // An IPv6 client controls addresses within its /64; rotating host bits
        // must not bypass the attempt budget or fill the address table.
        let ip = match ip.to_canonical() {
            IpAddr::V6(ip) => IpAddr::V6((u128::from(ip) & (u128::MAX << 64)).into()),
            ip => ip,
        };
        // Fixed windows, retained across reconnects. Expired records are removed
        // only at admission, never by a background sweep or on a data frame.
        self.rates
            .retain(|_, rate| now.saturating_sub(rate.started) < IP_WINDOW);
        if !self.rates.contains_key(&ip) && self.rates.len() >= self.limits.ip_records {
            return Err(Rejection::cap(Refusal::RateLimit, "ip_records"));
        }
        let rate = self.rates.entry(ip).or_insert(Rate {
            started: now,
            count: 0,
        });
        if rate.count >= self.limits.connections_per_ip {
            return Err(Rejection::cap(Refusal::RateLimit, "connections_per_ip"));
        }
        rate.count += 1;
        if self.peers.len() >= self.limits.connections {
            return Err(Rejection::cap(Refusal::ConnectionLimit, "connections"));
        }
        self.next_socket = self
            .next_socket
            .checked_add(1)
            .ok_or(Rejection::cap(Refusal::ConnectionLimit, "socket_ids"))?;
        let id = self.next_socket;
        self.peers.insert(
            id,
            Peer {
                role: Role::Pending,
                output,
                data_capacity: Arc::new(Semaphore::new(MAX_QUEUED_FRAMES)),
                control_capacity: Arc::new(Semaphore::new(self.limits.queued_controls)),
                departure_capacity: None,
                end,
                bytes: 0,
            },
        );
        Ok(id)
    }

    pub fn register(
        &mut self,
        id: u64,
        hello: DeviceHello,
        now: Duration,
    ) -> Result<(), Rejection> {
        if self.rooms.contains_key(&hello.room) {
            return Err(Rejection::new(Refusal::RoomOccupied, "room_occupied"));
        }
        self.usage.retain(|room, usage| {
            self.rooms.contains_key(room) || now.saturating_sub(usage.started) < DAY
        });
        if !self.usage.contains_key(&hello.room) && self.usage.len() >= self.limits.room_records {
            return Err(Rejection::cap(Refusal::ConnectionLimit, "room_records"));
        }
        self.usage.entry(hello.room.clone()).or_insert(Usage {
            started: now,
            bytes: 0,
        });
        self.peer_mut(id)?.role = Role::Device(hello.room.clone());
        self.rooms.insert(
            hello.room,
            Room {
                device: id,
                protocol: hello.phone_protocol,
                next_connection: 0,
                phones: HashMap::new(),
            },
        );
        self.control(
            id,
            ServerMessage::Registered {
                push: self.push_enabled,
            },
        )
    }

    pub fn join(&mut self, id: u64, room: String) -> Result<(), Rejection> {
        let entry = self
            .rooms
            .get_mut(&room)
            .ok_or(Rejection::new(Refusal::DeviceOffline, "device_offline"))?;
        if entry.phones.len() >= self.limits.phones_per_room {
            return Err(Rejection::cap(Refusal::RoomLimit, "phones_per_room"));
        }
        let connection = entry
            .next_connection
            .checked_add(1)
            .ok_or(Rejection::cap(Refusal::RoomLimit, "connection_ids"))?;
        let device = entry.device;
        let protocol = entry.protocol;
        // A failed announcement refuses the joining socket without changing
        // membership or consuming a connection ID on the device.
        // Reserve the departure too: repeated joins/leaves cannot consume the
        // last slot needed to tell the device an existing phone went away.
        let departure = self
            .peers
            .get(&device)
            .ok_or(invalid())?
            .control_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Rejection::cap(Refusal::ConnectionLimit, "control_queue"))?;
        self.control(device, ServerMessage::PeerJoined { connection })?;
        let entry = self.rooms.get_mut(&room).ok_or(invalid())?;
        entry.next_connection = connection;
        entry.phones.insert(connection, id);
        let peer = self.peer_mut(id)?;
        peer.role = Role::Phone(room, connection);
        peer.departure_capacity = Some(departure);
        self.control(
            id,
            ServerMessage::Joined {
                connection,
                phone_protocol: protocol,
            },
        )
    }

    pub fn forward(
        &mut self,
        id: u64,
        bytes: Vec<u8>,
        now: Duration,
    ) -> Result<Vec<Event>, Rejection> {
        let frame = Frame::decode(&bytes).map_err(crate::protocol_error)?;
        let role = self.role(id)?;
        let sending_phone = matches!(role, Role::Phone(..));
        let (room, target) = match role {
            Role::Phone(room, connection) if connection == frame.connection => {
                let device = self.rooms.get(&room).ok_or(invalid())?.device;
                (room, Some(device))
            }
            Role::Device(room) => {
                let peer = self
                    .rooms
                    .get(&room)
                    .ok_or(invalid())?
                    .phone(frame.connection)?;
                (room, peer)
            }
            _ => return Err(invalid()),
        };
        let usage = self.usage.get_mut(&room).ok_or(invalid())?;
        if now.saturating_sub(usage.started) >= DAY {
            *usage = Usage {
                started: now,
                bytes: 0,
            };
        }
        let size = bytes.len() as u64;
        if size
            > self
                .limits
                .bytes_per_room_per_day
                .saturating_sub(usage.bytes)
        {
            return Err(Rejection::cap(
                Refusal::DailyLimit,
                "bytes_per_room_per_day",
            ));
        }
        usage.bytes += size;
        self.peer_mut(id)?.bytes += size;
        match target {
            Some(target) => match self.send(target, Message::Binary(bytes.into())) {
                Ok(()) => Ok(Vec::new()),
                // A phone cannot evict the device by flooding its queue. Its
                // refusal releases membership and queues PeerLeft separately.
                Err(error) if sending_phone => Err(error),
                // A slow phone cannot evict the sending device either.
                Err(error) => Ok(self.remove(target, End::Refused(error))),
            },
            // A response may race the phone's departure. IDs are never reused,
            // so a retired peer is distinct from a foreign/future peer.
            None => self
                .control(
                    id,
                    ServerMessage::PeerLeft {
                        connection: frame.connection,
                    },
                )
                .map(|()| Vec::new()),
        }
    }

    /// Counts one push request against the room's and the relay's caps.
    /// Every request that gets here is counted, a malformed one included, so a
    /// room cannot spend the relay without a ceiling (`PushUsage::spend`).
    /// `Err` names the cap, and is a `rate_limited` result rather than a
    /// closed socket.
    pub fn push_cap(&mut self, room: &str, now: Duration) -> Result<(), &'static str> {
        let second = self.push_second.get_or_insert(Rate {
            started: now,
            count: 0,
        });
        if now.saturating_sub(second.started) >= Duration::from_secs(1) {
            *second = Rate {
                started: now,
                count: 0,
            };
        }
        if second.count >= self.push.per_second {
            // The relay's own cap is counted first and not spent by a refusal,
            // so one noisy room cannot hold the whole relay at its ceiling.
            return Err("pushes_per_second");
        }
        let room_usage = self.pushes.entry(room.to_owned()).or_default();
        room_usage.spend(now, self.push)?;
        second.count += 1;
        Ok(())
    }

    /// The room a peer belongs to, for a message the peer is about to answer.
    pub fn room_of(&self, id: u64) -> Option<String> {
        self.room(id)
    }

    pub fn set_push_limits(&mut self, push: PushLimits) {
        self.push = push;
    }

    /// Whether the peer is a room's device socket, which alone may ask for a
    /// push. A phone socket keeps its existing closed refusal.
    pub fn is_device(&self, id: u64) -> bool {
        matches!(
            self.peers.get(&id).map(|peer| &peer.role),
            Some(Role::Device(_))
        )
    }

    pub fn close_phone(&mut self, id: u64, connection: u32) -> Result<Vec<Event>, Rejection> {
        let target = match self.role(id)? {
            Role::Device(room) => self.rooms.get(&room).ok_or(invalid())?.phone(connection)?,
            Role::Phone(_, own) if own == connection => Some(id),
            _ => return Err(invalid()),
        };
        Ok(match target {
            Some(target) => self.remove(target, End::PeerClosed),
            None => Vec::new(), // Closing an already retired peer is complete.
        })
    }

    pub fn role(&self, id: u64) -> Result<Role, Rejection> {
        self.peers.get(&id).map(|p| p.role.clone()).ok_or(invalid())
    }
    pub fn room(&self, id: u64) -> Option<String> {
        self.peers
            .get(&id)
            .and_then(|p| p.role.room().map(str::to_owned))
    }
    fn peer_mut(&mut self, id: u64) -> Result<&mut Peer, Rejection> {
        self.peers.get_mut(&id).ok_or(invalid())
    }
    fn send(&self, id: u64, message: Message) -> Result<(), Rejection> {
        let peer = self.peers.get(&id).ok_or(invalid())?;
        let (capacity, cap) = if matches!(message, Message::Binary(_)) {
            (&peer.data_capacity, "outbound_queue")
        } else {
            (&peer.control_capacity, "control_queue")
        };
        let capacity = capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Rejection::cap(Refusal::ConnectionLimit, cap))?;
        peer.output
            .try_send(Outbound { message, capacity })
            .map_err(|_| Rejection::cap(Refusal::ConnectionLimit, cap))
    }
    pub fn control(&self, id: u64, message: ServerMessage) -> Result<(), Rejection> {
        self.send(
            id,
            Message::Text(message.encode().map_err(crate::protocol_error)?.into()),
        )
    }

    pub fn remove(&mut self, id: u64, end: End) -> Vec<Event> {
        let Some(mut peer) = self.peers.remove(&id) else {
            return Vec::new();
        };
        peer.end.send_replace(Some(end));
        let mut event = Event::new("peer_closed", peer.role.room().map(str::to_owned), None);
        event.size = Some(peer.bytes);
        let mut events = vec![event];
        if let End::Refused(error) = end {
            events.push(Event::new(
                error.event,
                peer.role.room().map(str::to_owned),
                error.cap,
            ));
        }
        match peer.role {
            Role::Pending => {}
            Role::Device(room) => {
                if let Some(entry) = self.rooms.remove(&room) {
                    for phone in entry.phones.values() {
                        events.extend(self.remove(
                            *phone,
                            End::Refused(Rejection::new(Refusal::DeviceOffline, "device_offline")),
                        ));
                    }
                }
            }
            Role::Phone(room, connection) => {
                if let Some(entry) = self.rooms.get_mut(&room) {
                    entry.phones.remove(&connection);
                    let device = entry.device;
                    let Some(capacity) = peer.departure_capacity.take() else {
                        events.push(Event::new("departure_capacity_missing", Some(room), None));
                        events.extend(self.remove(
                            device,
                            End::Refused(Rejection::new(
                                Refusal::InvalidMessage,
                                "lifecycle_failed",
                            )),
                        ));
                        return events;
                    };
                    let result = ServerMessage::PeerLeft { connection }
                        .encode()
                        .map_err(crate::protocol_error)
                        .and_then(|text| {
                            self.peers
                                .get(&device)
                                .ok_or(invalid())?
                                .output
                                .try_send(Outbound {
                                    message: Message::Text(text.into()),
                                    capacity,
                                })
                                .map_err(|_| {
                                    Rejection::cap(Refusal::ConnectionLimit, "control_queue")
                                })
                        });
                    if let Err(error) = result {
                        // A device that cannot receive lifecycle metadata must
                        // reconnect: keeping it alive would leave a ghost peer.
                        events.extend(self.remove(device, End::Refused(error)));
                    }
                }
            }
        }
        events
    }
    pub fn restart(&mut self) -> Vec<Event> {
        self.stopping = true;
        let peers = std::mem::take(&mut self.peers);
        self.rooms.clear();
        self.usage.clear();
        self.rates.clear();
        self.pushes.clear();
        self.push_second = None;
        peers
            .into_values()
            .map(|peer| {
                peer.end.send_replace(Some(End::Restart));
                Event::new("peer_closed", peer.role.room().map(str::to_owned), None)
            })
            .collect()
    }
}
fn invalid() -> Rejection {
    Rejection::new(Refusal::InvalidMessage, "invalid_routing")
}
