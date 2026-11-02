use crate::{event::Event, limits::*};
use hide_relay_protocol::{
    admission::{DeviceHello, Refusal, ServerMessage},
    frame::Frame,
};
use std::{collections::HashMap, net::IpAddr, time::Duration};
use tokio::sync::{mpsc, watch};
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
struct Peer {
    role: Role,
    output: mpsc::Sender<Message>,
    end: watch::Sender<Option<End>>,
    bytes: u64,
}
struct Room {
    device: u64,
    protocol: u32,
    next_connection: u32,
    phones: HashMap<u32, u64>,
}
struct Usage {
    started: Duration,
    bytes: u64,
}
struct Rate {
    started: Duration,
    count: usize,
}

pub(crate) struct State {
    stopping: bool,
    limits: Limits,
    next_socket: u64,
    peers: HashMap<u64, Peer>,
    rooms: HashMap<String, Room>,
    usage: HashMap<String, Usage>,
    rates: HashMap<IpAddr, Rate>,
}
impl State {
    pub fn new(limits: Limits) -> Self {
        Self {
            stopping: false,
            limits,
            next_socket: 0,
            peers: HashMap::new(),
            rooms: HashMap::new(),
            usage: HashMap::new(),
            rates: HashMap::new(),
        }
    }

    pub fn reserve(
        &mut self,
        ip: IpAddr,
        now: Duration,
        output: mpsc::Sender<Message>,
        end: watch::Sender<Option<End>>,
    ) -> Result<u64, Rejection> {
        if self.stopping {
            return Err(Rejection::new(Refusal::Timeout, "server_stopping"));
        }
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
        self.control(id, ServerMessage::Registered)
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
        entry.next_connection = connection;
        let device = entry.device;
        let protocol = entry.protocol;
        entry.phones.insert(connection, id);
        self.peer_mut(id)?.role = Role::Phone(room, connection);
        self.control(device, ServerMessage::PeerJoined { connection })?;
        self.control(
            id,
            ServerMessage::Joined {
                connection,
                phone_protocol: protocol,
            },
        )
    }

    pub fn forward(&mut self, id: u64, bytes: Vec<u8>, now: Duration) -> Result<(), Rejection> {
        let frame = Frame::decode(&bytes).map_err(crate::protocol_error)?;
        let (room, target) = match self.role(id)? {
            Role::Phone(room, connection) if connection == frame.connection => {
                let device = self.rooms.get(&room).ok_or(invalid())?.device;
                (room, device)
            }
            Role::Device(room) => {
                let peer = *self
                    .rooms
                    .get(&room)
                    .and_then(|r| r.phones.get(&frame.connection))
                    .ok_or(invalid())?;
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
        self.send(target, Message::Binary(bytes.into()))
    }

    pub fn close_phone(&mut self, id: u64, connection: u32) -> Result<Vec<Event>, Rejection> {
        let target = match self.role(id)? {
            Role::Device(room) => *self
                .rooms
                .get(&room)
                .and_then(|r| r.phones.get(&connection))
                .ok_or(invalid())?,
            Role::Phone(_, own) if own == connection => id,
            _ => return Err(invalid()),
        };
        Ok(self.remove(target, End::PeerClosed))
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
        self.peers
            .get(&id)
            .ok_or(invalid())?
            .output
            .try_send(message)
            .map_err(|_| Rejection::cap(Refusal::ConnectionLimit, "outbound_queue"))
    }
    pub fn control(&self, id: u64, message: ServerMessage) -> Result<(), Rejection> {
        self.send(
            id,
            Message::Text(message.encode().map_err(crate::protocol_error)?.into()),
        )
    }

    pub fn remove(&mut self, id: u64, end: End) -> Vec<Event> {
        let Some(peer) = self.peers.remove(&id) else {
            return Vec::new();
        };
        peer.end.send_replace(Some(end));
        let mut event = Event::new("peer_closed", peer.role.room().map(str::to_owned), None);
        event.size = Some(peer.bytes);
        let mut events = vec![event];
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
                    if let Err(error) = self.control(device, ServerMessage::PeerLeft { connection })
                    {
                        // A device that cannot receive lifecycle metadata must
                        // reconnect: keeping it alive would leave a ghost peer.
                        events.push(Event::new(error.event, Some(room), error.cap));
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
