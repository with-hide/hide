use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::*;
use crate::peer_host::fixture::{Machine, paired, within};

fn slot() -> Slot {
    Slot {
        last: Mutex::new(Instant::now()),
        reclaim: Notify::new(),
    }
}

/// Whether `slot` was told to give its place.
async fn reclaimed(slot: &Slot) -> bool {
    tokio::time::timeout(Duration::ZERO, slot.reclaim.notified())
        .await
        .is_ok()
}

// A connection past the cap takes the place of the tunnel quiet longest
// once it has moved nothing for a minute; a tunnel that carried bytes keeps
// its place (a dev server's quiet hot-reload socket is the one to go).
#[tokio::test(start_paused = true)]
async fn a_seventeenth_tunnel_takes_the_quietest_place_and_the_busy_one_stays() {
    let places = Arc::new(Places::default());
    let mut held = Vec::new();
    for _ in 0..PLACES {
        held.push(places.take().await.expect("a free place"));
    }
    tokio::time::advance(QUIET_FOR).await;
    held[0].slot.touch();
    let started = Instant::now();
    let seventeenth = places.take().await.expect("the quietest place");
    assert_eq!(started.elapsed(), Duration::ZERO, "it waited for nothing");
    assert!(
        !reclaimed(&held[0].slot).await,
        "the busy tunnel keeps its place"
    );
    assert!(
        reclaimed(&held[1].slot).await,
        "the quietest gave its place"
    );
    assert_eq!(lock(&places.inner).held.len(), PLACES);
    drop(seventeenth);
    drop(held);
    assert!(lock(&places.inner).held.is_empty());
}

// With every place busy, a connection waits for one to free, at most
// `PLACE_WITHIN`, and at most `MAX_WAITING` wait: one more is refused at once.
#[tokio::test(start_paused = true)]
async fn with_every_place_busy_a_connection_waits_its_bound_and_one_past_the_waiting_is_refused() {
    let places = Arc::new(Places::default());
    let mut held = Vec::new();
    for _ in 0..PLACES {
        held.push(places.take().await.unwrap());
    }
    let waiting: Vec<_> = (0..MAX_WAITING)
        .map(|_| {
            let places = Arc::clone(&places);
            tokio::spawn(async move { places.take().await })
        })
        .collect();
    tokio::task::yield_now().await;
    while lock(&places.inner).waiting < MAX_WAITING {
        tokio::task::yield_now().await;
    }
    let started = Instant::now();
    assert!(places.take().await.is_none(), "one past the waiting");
    assert_eq!(started.elapsed(), Duration::ZERO);
    // A freed place goes to one of them; the rest give up at their bound.
    drop(held.pop());
    let mut placed = Vec::new();
    for waiter in waiting {
        placed.extend(waiter.await.unwrap());
    }
    assert_eq!(placed.len(), 1);
    assert_eq!(lock(&places.inner).waiting, 0);
}

/// A tunnel's two ends in memory: the page's connection to the listener,
/// and the host's end of the stream.
struct Ends {
    page: DuplexStream,
    local: DuplexStream,
    host: DuplexStream,
    near: DuplexStream,
}

fn ends() -> Ends {
    let (page, local) = tokio::io::duplex(4 * CHUNK);
    let (host, near) = tokio::io::duplex(4 * CHUNK);
    Ends {
        page,
        local,
        host,
        near,
    }
}

/// Reads and drops everything `stream` gets until it closes.
fn drain(mut stream: impl AsyncRead + Unpin + Send + 'static) -> tokio::task::JoinHandle<u64> {
    tokio::spawn(async move {
        let mut buffer = vec![0_u8; CHUNK];
        let mut total = 0;
        while let Ok(read) = stream.read(&mut buffer).await {
            if read == 0 {
                break;
            }
            total += read as u64;
        }
        total
    })
}

/// Writes chunks into `stream` until it fails.
fn flood(mut stream: impl AsyncWrite + Unpin + Send + 'static) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let chunk = vec![7_u8; CHUNK];
        while stream.write_all(&chunk).await.is_ok() {}
    })
}

// A long stream on one tunnel is held too: each chunk is reserved before it
// is written, so a host streaming without end stops at the budget, and the
// day's counter never passes it.
#[tokio::test]
async fn one_tunnel_streaming_past_the_budget_ends_and_the_counter_never_passes_it() {
    let budget = RelayBudget::new(3 * CHUNK as u64 + 100);
    let Ends {
        page,
        mut local,
        host,
        near,
    } = ends();
    let (host_read, host_write) = tokio::io::split(host);
    let (near_read, near_write) = tokio::io::split(near);
    let _page = drain(page);
    let _host_in = drain(host_read);
    let _host_out = flood(host_write);
    let relay = || true;
    let piped = within(
        "the tunnel",
        pipe(&mut local, near_read, near_write, &budget, &relay, &slot()),
    )
    .await;
    assert_eq!(piped, Piped::Budget);
    assert!(budget.used() <= 3 * CHUNK as u64 + 100);
    assert!(budget.spent());
    assert!(!budget.reserve(1), "the pair's next tunnel is refused too");
}

// Sixteen tunnels of one pair sending and receiving at once share the one
// counter, and it ends no higher than the budget.
#[tokio::test]
async fn sixteen_tunnels_sending_and_receiving_at_once_end_with_the_counter_at_most_the_budget() {
    let limit = 40 * CHUNK as u64;
    let budget = Arc::new(RelayBudget::new(limit));
    let mut tunnels = Vec::new();
    for _ in 0..PLACES {
        let budget = Arc::clone(&budget);
        tunnels.push(tokio::spawn(async move {
            let Ends {
                page,
                mut local,
                host,
                near,
            } = ends();
            let (page_read, page_write) = tokio::io::split(page);
            let (host_read, host_write) = tokio::io::split(host);
            let (near_read, near_write) = tokio::io::split(near);
            let _ = (
                drain(page_read),
                flood(page_write),
                drain(host_read),
                flood(host_write),
            );
            let relay = || true;
            pipe(&mut local, near_read, near_write, &budget, &relay, &slot()).await
        }));
    }
    for tunnel in tunnels {
        assert_eq!(within("a tunnel", tunnel).await.unwrap(), Piped::Budget);
    }
    assert!(budget.used() <= limit, "{} > {limit}", budget.used());
}

// A direct path is never counted or ended; a tunnel that moves to the relay
// reserves from its next chunk and stops at the budget.
#[tokio::test]
async fn a_direct_tunnel_passes_the_budget_and_one_that_moves_to_the_relay_is_held() {
    let budget = Arc::new(RelayBudget::new(4 * CHUNK as u64));
    let Ends {
        mut page,
        local,
        host,
        near,
    } = ends();
    let (host_read, mut host_write) = tokio::io::split(host);
    let (near_read, near_write) = tokio::io::split(near);
    let on_relay = Arc::new(AtomicBool::new(false));
    let tunnel = {
        let (budget, on_relay) = (Arc::clone(&budget), Arc::clone(&on_relay));
        tokio::spawn(async move {
            let mut local = local;
            let relay = move || on_relay.load(Ordering::SeqCst);
            pipe(&mut local, near_read, near_write, &budget, &relay, &slot()).await
        })
    };
    let _host_in = drain(host_read);
    let chunk = vec![1_u8; CHUNK];
    let mut back = vec![0_u8; CHUNK];
    for _ in 0..16 {
        host_write.write_all(&chunk).await.unwrap();
        page.read_exact(&mut back).await.unwrap();
    }
    assert_eq!(budget.used(), 0, "direct bytes are not counted");
    on_relay.store(true, Ordering::SeqCst);
    let _page = drain(page);
    let _host_out = flood(host_write);
    assert_eq!(within("the tunnel", tunnel).await.unwrap(), Piped::Budget);
    assert!(budget.used() <= 4 * CHUNK as u64);
}

// A host whose build has no handler for the kind takes the stream and
// closes it with nothing written back: the tunnel says so, and the page
// gets an answer rather than a dropped connection.
#[tokio::test]
async fn a_stream_the_host_drops_unanswered_is_told_apart() {
    let budget = RelayBudget::new(RELAY_BUDGET);
    let Ends {
        page: _page,
        mut local,
        host,
        near,
    } = ends();
    let (near_read, near_write) = tokio::io::split(near);
    drop(host);
    let relay = || false;
    let piped = within(
        "the tunnel",
        pipe(&mut local, near_read, near_write, &budget, &relay, &slot()),
    )
    .await;
    assert_eq!(piped, Piped::Unanswered);
}

fn basic(secret: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("hide:{secret}"))
}

#[test]
fn only_the_listener_s_own_secret_authorizes_a_connection() {
    let secret = "00112233445566778899aabbccddeeff";
    let head = |auth: &str| {
        format!("GET http://example.test/ HTTP/1.1\r\nHost: example.test\r\n{auth}\r\n")
            .into_bytes()
    };
    assert!(authorized(
        &head(&format!("Proxy-Authorization: Basic {}\r\n", basic(secret))),
        secret
    ));
    assert!(authorized(
        &head(&format!("proxy-authorization:basic {}\r\n", basic(secret))),
        secret
    ));
    assert!(!authorized(
        &head(&format!(
            "Proxy-Authorization: Basic {}\r\n",
            basic("ffeeddccbbaa99887766554433221100")
        )),
        secret
    ));
    assert!(!authorized(
        &head(&format!(
            "Proxy-Authorization: Bearer {}\r\n",
            basic(secret)
        )),
        secret
    ));
    assert!(!authorized(&head(""), secret));
    // The header after the head's end is the body's, not the head's.
    let late = format!(
        "GET http://example.test/ HTTP/1.1\r\n\r\nProxy-Authorization: Basic {}\r\n",
        basic(secret)
    );
    assert!(!authorized(late.as_bytes(), secret));
}

#[tokio::test]
async fn a_head_past_its_bound_is_refused() {
    let (mut page, mut local) = tokio::io::duplex(4 * MAX_HEAD);
    page.write_all(&vec![b'a'; MAX_HEAD + 2]).await.unwrap();
    assert!(read_head(&mut local).await.is_err());
}

/// An origin on this machine's loopback that answers every request `ok`
/// and keeps the heads it got.
async fn origin() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let heads = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&heads);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let kept = Arc::clone(&kept);
            tokio::spawn(async move {
                loop {
                    let Ok(head) = read_head(&mut stream).await else {
                        return;
                    };
                    lock(&kept).push(String::from_utf8_lossy(&head).into_owned());
                    if stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
    (port, heads)
}

/// The first response head on `stream`.
async fn answer_of(stream: &mut (impl AsyncRead + Unpin)) -> String {
    String::from_utf8_lossy(&within("an answer", read_head(stream)).await.unwrap()).into_owned()
}

/// One connection to the host's proxy, as the guest's tunnel carries it.
fn proxy_stream(hosts: HashMap<String, IpAddr>) -> DuplexStream {
    let (guest, host) = tokio::io::duplex(4 * CHUNK);
    tokio::spawn(serve_proxy(host, 1, Arc::new(hosts)));
    guest
}

// The host's proxy sends an absolute-form request to its origin in origin
// form, without the proxy's credentials or the connection's own headers.
#[tokio::test]
async fn the_host_proxy_sends_a_request_on_without_the_proxy_s_headers() {
    let (port, heads) = origin().await;
    let mut guest = proxy_stream(HashMap::new());
    guest
        .write_all(
            format!("GET http://127.0.0.1:{port}/x?y=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nProxy-Authorization: Basic c2VjcmV0\r\nProxy-Connection: keep-alive\r\nX-Kept: 1\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    assert!(answer_of(&mut guest).await.starts_with("HTTP/1.1 200"));
    let head = lock(&heads)[0].to_ascii_lowercase();
    assert!(head.starts_with("get /x?y=1 http/1.1"), "{head}");
    assert!(head.contains("x-kept: 1"), "{head}");
    assert!(
        !head.contains("proxy-authorization") && !head.contains("proxy-connection"),
        "{head}"
    );
}

// `CONNECT` dials on the host and joins the two, so TLS and WebSockets
// pass whole; a name only the host's resolver answers is the host's.
#[tokio::test]
async fn connect_dials_the_host_s_network_and_joins_the_two() {
    let (port, heads) = origin().await;
    let hosts = HashMap::from([("only-host.test".to_owned(), IpAddr::from([127, 0, 0, 1]))]);
    let mut guest = proxy_stream(hosts);
    guest
        .write_all(
            format!(
                "CONNECT only-host.test:{port} HTTP/1.1\r\nHost: only-host.test:{port}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    assert!(answer_of(&mut guest).await.starts_with("HTTP/1.1 200"));
    guest
        .write_all(b"GET /tunneled HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    assert!(answer_of(&mut guest).await.starts_with("HTTP/1.1 200"));
    assert!(lock(&heads)[0].starts_with("GET /tunneled"));
}

// A request that names no origin is refused, and a dial that fails is a
// bad gateway rather than a hang.
#[tokio::test]
async fn the_host_proxy_refuses_an_origin_form_request_and_answers_a_failed_dial() {
    let mut guest = proxy_stream(HashMap::new());
    guest
        .write_all(b"GET /relative HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    assert!(answer_of(&mut guest).await.starts_with("HTTP/1.1 400"));
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let mut guest = proxy_stream(HashMap::new());
    guest
        .write_all(
            format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    assert!(answer_of(&mut guest).await.starts_with("HTTP/1.1 502"));
}

#[test]
fn test_host_names_parse_as_pairs_or_not_at_all() {
    let parsed = parse_hosts("only-host.test=127.0.0.1, Other.Test=::1").unwrap();
    assert_eq!(
        parsed.get("only-host.test"),
        Some(&IpAddr::from([127, 0, 0, 1]))
    );
    assert!(parsed.contains_key("other.test"));
    assert!(parse_hosts("no-address").is_none());
    assert!(parse_hosts("x=not-an-ip").is_none());
    assert!(parse_hosts("http://x=127.0.0.1").is_none());
}

/// A guest paired to a host whose proxy serves `page` streams, the guest's
/// half over its own endpoint, and the host machine's device id there.
async fn pair() -> (Machine, Machine, Arc<ScreenNet>, PeerKey) {
    let host = Machine::new("windows-pc");
    let guest = Machine::new("macbook");
    ScreenNet::new(Unpaired, HashMap::new()).serve(&host.host);
    let link = paired(&host, &guest).await;
    let net = ScreenNet::new(Arc::clone(&guest.host), HashMap::new());
    (host, guest, net, link.key())
}

/// The host's own half reaches no machine.
struct Unpaired;

impl Reach for Unpaired {
    fn link(&self, _device: &str, _key: &PeerKey) -> Result<Link, &'static str> {
        Err("not_paired")
    }
}

/// A page's connection to the guest's listener at `port`.
async fn page(port: u16) -> TcpStream {
    TcpStream::connect(("127.0.0.1", port)).await.unwrap()
}

// The page's requests leave through the host: with the listener's secret a
// request reaches an origin on the host's loopback; without it the page is
// asked for it and no stream is opened. The same owner keeps the same port
// and secret, and one it lets go closes.
#[tokio::test]
async fn a_page_s_request_rides_a_page_stream_to_the_host_s_network_only_with_its_secret() {
    let (_host, _guest, net, key) = pair().await;
    let (port, heads) = origin().await;
    let pid = i32::try_from(std::process::id()).unwrap();
    let ensured = net.ensure("windows-pc", key, pid).await.unwrap();
    assert_eq!(ensured.state, "ready");
    assert_eq!(
        net.ensure("windows-pc", key, pid).await.unwrap(),
        ensured,
        "the same listener for its owner"
    );

    let mut refused = page(ensured.port).await;
    refused
        .write_all(format!("GET http://127.0.0.1:{port}/a HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let answer = answer_of(&mut refused).await;
    assert!(answer.starts_with("HTTP/1.1 407"), "{answer}");
    assert!(
        answer.contains("Proxy-Authenticate: Basic realm=\"hide\""),
        "{answer}"
    );

    let mut carried = page(ensured.port).await;
    carried
        .write_all(
            format!(
                "GET http://127.0.0.1:{port}/b HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nProxy-Authorization: Basic {}\r\n\r\n",
                basic(&ensured.secret)
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let answer = answer_of(&mut carried).await;
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    let heads = lock(&heads).clone();
    assert_eq!(
        heads.len(),
        1,
        "the refused connection reached nothing: {heads:?}"
    );
    assert!(heads[0].starts_with("GET /b"));

    assert!(net.release("windows-pc", key, pid));
    within("the listener to close", async {
        while TcpStream::connect(("127.0.0.1", ensured.port))
            .await
            .is_ok()
        {
            tokio::time::sleep(Duration::from_millis(20)).await; // wait-ok: the released listener's port refuses connections
        }
    })
    .await;
}

// A machine not paired under the key opens nothing; with every place held
// a new owner is refused and every held listener keeps its port; a listener
// whose owner process ended closes at its next check.
#[tokio::test]
async fn listeners_are_bounded_and_end_with_their_owner_or_their_pairing() {
    let (_host, _guest, net, key) = pair().await;
    let own = i32::try_from(std::process::id()).unwrap();
    assert_eq!(net.ensure("another-pc", key, own).await, Err("not_paired"));
    assert_eq!(net.listening(), 0);
    let mut owners: Vec<std::process::Child> = (0..=MAX_LISTENERS)
        .map(|_| {
            std::process::Command::new("sleep")
                .arg("60")
                .spawn()
                .unwrap()
        })
        .collect();
    let mut held = Vec::new();
    for owner in &owners[..MAX_LISTENERS] {
        held.push(
            net.ensure("windows-pc", key, owner.id() as i32)
                .await
                .unwrap(),
        );
    }
    assert_eq!(
        net.ensure("windows-pc", key, owners[MAX_LISTENERS].id() as i32)
            .await,
        Err("net_limit")
    );
    for (owner, listener) in owners.iter().zip(&held) {
        assert_eq!(
            net.ensure("windows-pc", key, owner.id() as i32)
                .await
                .unwrap()
                .port,
            listener.port
        );
    }
    owners[0].kill().unwrap();
    owners[0].wait().unwrap();
    within("the gone owner's listener to close", async {
        while net.listening() == MAX_LISTENERS {
            tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the gone owner's listener is closed by its periodic check
        }
    })
    .await;
    for owner in &mut owners {
        let _ = owner.kill();
        let _ = owner.wait();
    }
}
