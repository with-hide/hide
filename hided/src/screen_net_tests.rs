use std::sync::atomic::{AtomicBool, AtomicUsize};
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

// A long stream on one tunnel is held too: a read is made only while the
// day's bytes are under the budget, so a host streaming without end stops
// at the budget, at most one chunk past it, and the pair's next tunnel is
// refused.
#[tokio::test]
async fn one_tunnel_streaming_past_the_budget_ends_at_most_one_chunk_over_and_the_next_is_refused()
{
    let limit = 3 * CHUNK as u64 + 100;
    let budget = RelayBudget::new(limit);
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
        pipe(
            &mut local,
            &[],
            near_read,
            near_write,
            &budget,
            &relay,
            &slot(),
            &|| {},
        ),
    )
    .await
    .0;
    assert_eq!(piped, Piped::Budget);
    assert!(budget.spent());
    assert!(
        budget.used() <= limit + CHUNK as u64,
        "{} past {limit}",
        budget.used()
    );

    // The next tunnel's head is refused before a byte of it is written.
    let Ends {
        page: _page,
        mut local,
        host,
        near,
    } = ends();
    let (host_read, _host_write) = tokio::io::split(host);
    let (near_read, near_write) = tokio::io::split(near);
    let host_got = drain(host_read);
    let used = budget.used();
    let next = within(
        "the next tunnel",
        pipe(
            &mut local,
            b"GET http://example.test/ HTTP/1.1\r\n\r\n",
            near_read,
            near_write,
            &budget,
            &relay,
            &slot(),
            &|| {},
        ),
    )
    .await;
    assert_eq!(next, (Piped::Budget, 0));
    assert_eq!(budget.used(), used);
    drop(local);
    assert_eq!(within("the host's end", host_got).await.unwrap(), 0);
}

// Sixteen tunnels of one pair sending and receiving at once share the one
// counter, and it ends no further past the budget than one chunk for each
// of their thirty-two copies.
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
            pipe(
                &mut local,
                &[],
                near_read,
                near_write,
                &budget,
                &relay,
                &slot(),
                &|| {},
            )
            .await
            .0
        }));
    }
    for tunnel in tunnels {
        assert_eq!(within("a tunnel", tunnel).await.unwrap(), Piped::Budget);
    }
    let most = limit + 2 * PLACES as u64 * CHUNK as u64;
    assert!(budget.used() <= most, "{} > {most}", budget.used());
}

// A direct path is never counted or ended; a tunnel that moves to the relay
// counts from its next read and stops at the budget.
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
            pipe(
                &mut local,
                &[],
                near_read,
                near_write,
                &budget,
                &relay,
                &slot(),
                &|| {},
            )
            .await
            .0
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
    assert!(budget.used() <= 5 * CHUNK as u64);
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
        pipe(
            &mut local,
            &[],
            near_read,
            near_write,
            &budget,
            &relay,
            &slot(),
            &|| {},
        ),
    )
    .await;
    assert_eq!(piped, (Piped::Unanswered, 0));

    // On the real transport the host's end closes both ways at once, so the
    // head's own write is what fails first; that is the same answer.
    let Ends {
        page: _page,
        mut local,
        host,
        near,
    } = ends();
    let (near_read, near_write) = tokio::io::split(near);
    drop(host);
    let head = b"GET http://example.test/ HTTP/1.1\r\n\r\n".repeat(4096);
    let piped = within(
        "the tunnel",
        pipe(
            &mut local,
            &head,
            near_read,
            near_write,
            &budget,
            &relay,
            &slot(),
            &|| {},
        ),
    )
    .await;
    assert_eq!(piped, (Piped::Unanswered, 0));
}

// A connection's head, and the body bytes read with it, are the first bytes
// the guest sends: they are counted on the relay before they are written,
// as every later read is, and once the day is spent no part of a head is
// written.
#[tokio::test]
async fn a_head_is_counted_before_it_is_written_and_none_is_written_once_the_day_is_spent() {
    let head = vec![b'h'; 3 * 1024];
    let relay = || true;
    let refused = RelayBudget::new(1024);
    refused.add(1024);
    let Ends {
        page: _page,
        mut local,
        host,
        near,
    } = ends();
    let (host_read, _host_write) = tokio::io::split(host);
    let (near_read, near_write) = tokio::io::split(near);
    let host_got = drain(host_read);
    let piped = within(
        "the tunnel",
        pipe(
            &mut local,
            &head,
            near_read,
            near_write,
            &refused,
            &relay,
            &slot(),
            &|| {},
        ),
    )
    .await;
    assert_eq!(piped, (Piped::Budget, 0));
    assert_eq!(refused.used(), 1024, "nothing of the head was counted");
    drop(local);
    assert_eq!(
        within("the host's end", host_got).await.unwrap(),
        0,
        "nothing of the head was written"
    );

    let counted = RelayBudget::new(8 * 1024);
    let Ends {
        page,
        mut local,
        host,
        near,
    } = ends();
    let (near_read, near_write) = tokio::io::split(near);
    drop(page);
    let host_got = tokio::spawn(async move {
        let (mut host_read, host_write) = tokio::io::split(host);
        let mut got = Vec::new();
        let _ = host_read.read_to_end(&mut got).await;
        drop(host_write);
        got.len()
    });
    within(
        "the tunnel",
        pipe(
            &mut local,
            &head,
            near_read,
            near_write,
            &counted,
            &relay,
            &slot(),
            &|| {},
        ),
    )
    .await;
    assert_eq!(
        within("the host's end", host_got).await.unwrap(),
        head.len()
    );
    assert_eq!(counted.used(), head.len() as u64);
}

// A connection whose task is dropped while it waits for a place (its
// listener released, its owner gone) gives its waiting count back, so the
// pair's next connections still wait rather than being refused at once.
#[tokio::test(start_paused = true)]
async fn waiters_whose_tasks_are_dropped_give_their_count_back() {
    let places = Arc::new(Places::default());
    let mut held = Vec::new();
    for _ in 0..PLACES {
        held.push(places.take().await.unwrap());
    }
    let waiting: Vec<_> = (0..MAX_WAITING)
        .map(|_| {
            let places = Arc::clone(&places);
            tokio::spawn(async move { places.take().await.is_some() })
        })
        .collect();
    while lock(&places.inner).waiting < MAX_WAITING {
        tokio::task::yield_now().await;
    }
    for waiter in &waiting {
        waiter.abort();
    }
    for waiter in waiting {
        let _ = waiter.await;
    }
    assert_eq!(lock(&places.inner).waiting, 0);
    let next = {
        let places = Arc::clone(&places);
        tokio::spawn(async move { places.take().await.is_some() })
    };
    while lock(&places.inner).waiting < 1 {
        tokio::task::yield_now().await;
    }
    drop(held.pop());
    assert!(next.await.unwrap(), "it waited and took the freed place");
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

// The bound holds the head with its blank line, whether or not it ended:
// one that ends past it is refused, and one that ends exactly at it is kept
// with the body bytes read beside it.
#[tokio::test]
async fn a_completed_head_past_its_bound_is_refused_and_one_at_it_keeps_its_body() {
    let start = b"GET http://example.test/ HTTP/1.1\r\nX-Pad: ".to_vec();
    let mut long = start.clone();
    long.extend(vec![b'a'; MAX_HEAD]);
    long.extend(b"\r\n\r\n");
    let (mut page, mut local) = tokio::io::duplex(4 * MAX_HEAD);
    page.write_all(&long).await.unwrap();
    assert!(read_head(&mut local).await.is_err());

    let mut exact = start;
    exact.extend(vec![b'a'; MAX_HEAD - exact.len() - 4]);
    exact.extend(b"\r\n\r\n");
    assert_eq!(exact.len(), MAX_HEAD);
    exact.extend(b"body");
    let (mut page, mut local) = tokio::io::duplex(4 * MAX_HEAD);
    page.write_all(&exact).await.unwrap();
    drop(page);
    let head = read_head(&mut local).await.expect("a head at the bound");
    assert!(head.len() >= MAX_HEAD);
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

    fn pairing(&self, _device: &str, _key: &PeerKey) -> Option<String> {
        None
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

/// Processes the listener tests own as listeners' owners: this test binary
/// run as `idle_owner`, which waits on its standard input, so the test needs
/// no program of the system's; every one is ended and reaped when the guard
/// goes, a failing assertion included.
struct Owners(Vec<std::process::Child>);

impl Owners {
    fn start(count: usize) -> Self {
        let mut owners = Self(Vec::new());
        for _ in 0..count {
            owners.0.push(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "screen_net::tests::idle_owner",
                        "--ignored",
                        "--quiet",
                    ])
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
        }
        owners
    }

    fn pid(&self, at: usize) -> i32 {
        i32::try_from(self.0[at].id()).unwrap()
    }

    fn end(&mut self, at: usize) {
        let _ = self.0[at].kill();
        let _ = self.0[at].wait();
    }
}

impl Drop for Owners {
    fn drop(&mut self) {
        for owner in &mut self.0 {
            let _ = owner.kill();
            let _ = owner.wait();
        }
    }
}

/// The process `Owners` starts: it ends when its standard input closes.
#[test]
#[ignore = "run only as the child of the listener tests"]
fn idle_owner() {
    let _ = std::io::Read::read(&mut std::io::stdin(), &mut [0_u8; 1]);
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
    let mut owners = Owners::start(MAX_LISTENERS + 1);
    let mut held = Vec::new();
    for at in 0..MAX_LISTENERS {
        held.push(net.ensure("windows-pc", key, owners.pid(at)).await.unwrap());
    }
    assert_eq!(
        net.ensure("windows-pc", key, owners.pid(MAX_LISTENERS))
            .await,
        Err("net_limit")
    );
    for (at, listener) in held.iter().enumerate() {
        assert_eq!(
            net.ensure("windows-pc", key, owners.pid(at))
                .await
                .unwrap()
                .port,
            listener.port
        );
    }
    owners.end(0);
    within("the gone owner's listener to close", async {
        while net.listening() == MAX_LISTENERS {
            tokio::time::sleep(Duration::from_millis(100)).await; // wait-ok: the gone owner's listener is closed by its periodic check
        }
    })
    .await;
}

/// The one listener `net` holds now.
fn only_listener(net: &ScreenNet) -> Arc<Listener> {
    let listeners = lock(&net.listeners);
    let [listener] = listeners.values().collect::<Vec<_>>()[..] else {
        panic!("one listener: {}", listeners.len())
    };
    Arc::clone(listener)
}

/// A request through the listener at `port` for an origin on this
/// machine's loopback, and the status line of its answer.
async fn through(port: u16, secret: &str, origin: u16, path: &str) -> String {
    let mut page = page(port).await;
    page.write_all(
        format!(
            "GET http://127.0.0.1:{origin}{path} HTTP/1.1\r\nHost: 127.0.0.1:{origin}\r\nProxy-Authorization: Basic {}\r\n\r\n",
            basic(secret)
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    answer_of(&mut page)
        .await
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

// A host that dropped a stream unanswered is a build to update on the
// connection it was seen on alone: the same host updated and connected
// again is asked afresh, and the first answer back clears the mark, through
// the same port.
#[tokio::test]
async fn an_unanswered_stream_names_its_connection_and_an_answer_clears_it() {
    let (_host, guest, net, key) = pair().await;
    let pid = i32::try_from(std::process::id()).unwrap();
    let ensured = net.ensure("windows-pc", key, pid).await.unwrap();
    let listener = only_listener(&net);
    let link = net.reach.link("windows-pc", &key).unwrap();
    let grant = guest
        .host
        .peers()
        .unwrap()
        .hosts()
        .into_iter()
        .find(|host| host.key == key)
        .unwrap()
        .grant;
    assert_eq!(
        ensured.pairing,
        pairing_of(&grant),
        "the pairing names the grant"
    );
    assert_eq!(ensured.pairing.len(), 32);
    *lock(&listener.unsupported) = Some(link.id().wrapping_add(1));
    assert_eq!(
        net.ensure("windows-pc", key, pid).await.unwrap().state,
        "ready",
        "another connection's mark"
    );
    *lock(&listener.unsupported) = Some(link.id());
    assert_eq!(
        net.ensure("windows-pc", key, pid).await.unwrap().state,
        "unsupported_there"
    );
    let (origin, _) = origin().await;
    assert_eq!(
        through(ensured.port, &ensured.secret, origin, "/answered").await,
        "HTTP/1.1 200 OK"
    );
    let again = net.ensure("windows-pc", key, pid).await.unwrap();
    assert_eq!((again.port, again.state), (ensured.port, "ready"));
}

// A failed accept fails that one connection: the listener and the port the
// pages point at stay, and the next connection is carried.
#[tokio::test]
async fn a_failed_accept_keeps_the_listener_and_its_port() {
    let (_host, _guest, net, key) = pair().await;
    let pid = i32::try_from(std::process::id()).unwrap();
    let ensured = net.ensure("windows-pc", key, pid).await.unwrap();
    only_listener(&net).fail_accepts.store(3, Ordering::SeqCst);
    let (origin, _) = origin().await;
    assert_eq!(
        through(ensured.port, &ensured.secret, origin, "/after").await,
        "HTTP/1.1 200 OK"
    );
    assert_eq!(net.listening(), 1);
    assert_eq!(
        net.ensure("windows-pc", key, pid).await.unwrap().port,
        ensured.port
    );
}

// The host counts its streams on its own side: a stream it refuses for the
// count is asked for again, and carried once one of the pair's ends there.
#[tokio::test]
async fn a_stream_the_host_refuses_for_its_count_is_asked_for_again_until_one_ends() {
    let (_host, _guest, net, key) = pair().await;
    let link = net.reach.link("windows-pc", &key).unwrap();
    let mut held = Vec::new();
    for _ in 0..PLACES {
        held.push(link.open(StreamKind::Page).await.unwrap());
    }
    assert!(matches!(
        link.open(StreamKind::Page).await,
        Err(hide_peer::peers::OpenError::Refused(
            hide_peer::protocol::stream::Status::Limit
        ))
    ));
    let asking = {
        let link = link.clone();
        tokio::spawn(async move { open_page(&link).await.is_ok() })
    };
    tokio::time::sleep(OPEN_AGAIN * 4).await; // wait-ok: the open has been refused at least once before a stream ends
    drop(held.pop());
    assert!(within("the stream after one ended", asking).await.unwrap());
}

// What is kept for a pair goes with its last listener unless it counted
// relay bytes today, and the budgets are held to `MAX_BUDGETS` with no
// allowance reset: a pair past it counts in the shared record.
#[tokio::test]
async fn a_pair_s_records_go_with_its_last_listener_and_today_s_bytes_stay() {
    let (_host, _guest, net, key) = pair().await;
    let pid = i32::try_from(std::process::id()).unwrap();
    net.ensure("windows-pc", key, pid).await.unwrap();
    let _ = net.places_of(key);
    let _ = net.budget_of(key);
    assert_eq!(net.kept(), (1, 1));
    net.release("windows-pc", key, pid);
    assert_eq!(net.kept(), (0, 0), "nothing counted: nothing kept");
    net.ensure("windows-pc", key, pid).await.unwrap();
    net.budget_of(key).add(10);
    net.release("windows-pc", key, pid);
    assert_eq!(net.kept().1, 1, "today's bytes stay");
    assert_eq!(net.budget_of(key).used(), 10);

    for n in 0..MAX_BUDGETS + 10 {
        let mut bytes = [1_u8; 32];
        bytes[..8].copy_from_slice(&(n as u64).to_le_bytes());
        net.budget_of(PeerKey::from_bytes(bytes)).add(1);
    }
    assert!(net.kept().1 <= MAX_BUDGETS + 1, "{}", net.kept().1);
    assert_eq!(net.budget_of(key).used(), 10, "no allowance was reset");
}

// A pair's places outlive its listener while a tunnel or a waiting
// connection still holds them, and go at the next ask once those end, so
// releasing a listener with work in flight and churning through pairs
// keeps no record of a finished pair.
#[tokio::test]
async fn a_pair_s_places_go_once_its_last_tunnel_ends_after_its_listener() {
    let (_host, _guest, net, key) = pair().await;
    let pid = i32::try_from(std::process::id()).unwrap();
    let ensured = net.ensure("windows-pc", key, pid).await.unwrap();
    let places = net.places_of(key);
    let mut held = Vec::new();
    for _ in 0..PLACES {
        held.push(places.take().await.unwrap());
    }
    drop(places);
    // A page's connection, past its secret, waits for a place.
    let mut waiting = page(ensured.port).await;
    waiting
        .write_all(
            format!(
                "GET http://127.0.0.1:9/ HTTP/1.1\r\nHost: 127.0.0.1:9\r\nProxy-Authorization: Basic {}\r\n\r\n",
                basic(&ensured.secret)
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let record = || Arc::clone(lock(&net.places).get(&key).expect("the pair's places"));
    within("the connection to wait for a place", async {
        while lock(&record().inner).waiting == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await; // wait-ok: the guest reads the head and starts waiting on its own task
        }
    })
    .await;

    assert!(net.release("windows-pc", key, pid));
    assert_eq!(
        net.kept().0,
        1,
        "held by its tunnels and the waiting connection"
    );
    drop(held);
    within("the pair's last holder to go", async {
        while Arc::strong_count(&record()) > 2 {
            tokio::time::sleep(Duration::from_millis(10)).await; // wait-ok: the released listener's tasks end when next polled
        }
    })
    .await;
    for n in 0..64_u8 {
        let _ = net.places_of(PeerKey::from_bytes([n.wrapping_add(1); 32]));
    }
    assert_eq!(net.kept().0, 1, "only the pair asked last is kept");
    assert!(!lock(&net.places).contains_key(&key));
}

/// A stream whose bytes written while `on_relay` is set are counted in
/// `relay`, and every byte written in `all`.
struct Watched<T> {
    inner: T,
    on_relay: Arc<AtomicBool>,
    relay: Arc<AtomicU64>,
    all: Arc<AtomicU64>,
}

impl<T> Watched<T> {
    fn new(
        inner: T,
        on_relay: &Arc<AtomicBool>,
        relay: &Arc<AtomicU64>,
        all: &Arc<AtomicU64>,
    ) -> Self {
        Self {
            inner,
            on_relay: Arc::clone(on_relay),
            relay: Arc::clone(relay),
            all: Arc::clone(all),
        }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Watched<T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Watched<T> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let this = self.get_mut();
        let relay = this.on_relay.load(Ordering::SeqCst);
        let written = std::pin::Pin::new(&mut this.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(count)) = written {
            this.all.fetch_add(count as u64, Ordering::SeqCst);
            if relay {
                this.relay.fetch_add(count as u64, Ordering::SeqCst);
            }
        }
        written
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

// R18 at a path move: sixteen tunnels streaming both ways on a direct path
// count nothing; once the path is the relay every read is counted before
// it is written, so what the tunnels write on the relay passes what was
// counted by at most the chunk each direction had read before the move:
// 2 x 16 x 64 KiB = 2 MiB.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tunnels_moving_from_direct_to_relay_mid_stream_overshoot_by_at_most_2_mib() {
    let limit = 8 * CHUNK as u64;
    let budget = Arc::new(RelayBudget::new(limit));
    let on_relay = Arc::new(AtomicBool::new(false));
    let relay = Arc::new(AtomicU64::new(0));
    let all = Arc::new(AtomicU64::new(0));
    let mut tunnels = Vec::new();
    let mut ends_kept = Vec::new();
    for _ in 0..PLACES {
        let Ends {
            page,
            local,
            host,
            near,
        } = ends();
        let (host_read, host_write) = tokio::io::split(host);
        let (page_read, page_write) = tokio::io::split(page);
        ends_kept.push((
            drain(host_read),
            drain(page_read),
            flood(host_write),
            flood(page_write),
        ));
        let near = Watched::new(near, &on_relay, &relay, &all);
        let mut local = Watched::new(local, &on_relay, &relay, &all);
        let (near_read, near_write) = tokio::io::split(near);
        let (budget, on_relay) = (Arc::clone(&budget), Arc::clone(&on_relay));
        tunnels.push(tokio::spawn(async move {
            let path = move || on_relay.load(Ordering::SeqCst);
            pipe(
                &mut local,
                &[],
                near_read,
                near_write,
                &budget,
                &path,
                &slot(),
                &|| {},
            )
            .await
            .0
        }));
    }
    within("the direct bytes", async {
        while all.load(Ordering::SeqCst) < PLACES as u64 * 32 * CHUNK as u64 {
            tokio::time::sleep(Duration::from_millis(5)).await; // wait-ok: each tunnel moves its first direct megabytes
        }
    })
    .await;
    assert_eq!(budget.used(), 0, "direct bytes are not counted");
    on_relay.store(true, Ordering::SeqCst);
    for tunnel in tunnels {
        assert_eq!(within("a tunnel", tunnel).await.unwrap(), Piped::Budget);
    }
    assert!(budget.used() <= limit + 2 * PLACES as u64 * CHUNK as u64);
    let past = relay.load(Ordering::SeqCst).saturating_sub(budget.used());
    assert!(
        past <= 2 * PLACES as u64 * CHUNK as u64,
        "{past} bytes written on the relay past what was counted"
    );
}

// A relay tunnel whose path moves to direct stops counting: it carries on
// past the budget and is never ended for it.
#[tokio::test]
async fn a_relay_tunnel_moving_to_direct_is_not_counted_or_ended() {
    let budget = Arc::new(RelayBudget::new(4 * CHUNK as u64));
    let Ends {
        mut page,
        local,
        host,
        near,
    } = ends();
    let (host_read, mut host_write) = tokio::io::split(host);
    let (near_read, near_write) = tokio::io::split(near);
    let on_relay = Arc::new(AtomicBool::new(true));
    let tunnel = {
        let (budget, on_relay) = (Arc::clone(&budget), Arc::clone(&on_relay));
        tokio::spawn(async move {
            let mut local = local;
            let path = move || on_relay.load(Ordering::SeqCst);
            pipe(
                &mut local,
                &[],
                near_read,
                near_write,
                &budget,
                &path,
                &slot(),
                &|| {},
            )
            .await
            .0
        })
    };
    let _host_in = drain(host_read);
    let chunk = vec![1_u8; CHUNK];
    let mut back = vec![0_u8; CHUNK];
    for _ in 0..2 {
        host_write.write_all(&chunk).await.unwrap();
        page.read_exact(&mut back).await.unwrap();
    }
    // Two chunks read on the relay, and each direction's waiting read
    // holding at most one more.
    assert_eq!(budget.used(), 2 * CHUNK as u64);
    on_relay.store(false, Ordering::SeqCst);
    for _ in 0..2 {
        host_write.write_all(&chunk).await.unwrap();
        page.read_exact(&mut back).await.unwrap();
    }
    // A read begun before the move counts what it takes after it, at most
    // one chunk; nothing read since is counted.
    let settled = budget.used();
    assert!(settled <= 3 * CHUNK as u64, "{settled}");
    for _ in 0..62 {
        host_write.write_all(&chunk).await.unwrap();
        page.read_exact(&mut back).await.unwrap();
    }
    assert_eq!(budget.used(), settled, "direct bytes are not counted");
    assert!(
        !tunnel.is_finished(),
        "a direct tunnel is not ended for the budget"
    );
    drop(host_write);
    drop(page);
    assert_eq!(within("the tunnel", tunnel).await.unwrap(), Piped::Done);
}

/// The direct transport's stream receive window: noq's default
/// (`TransportConfig::stream_receive_window`, `STREAM_RWND`, 12,500,000 /
/// 1000 x 100 bytes in noq-proto's `src/config/transport.rs`), which
/// `hide-peer-iroh` keeps.
const DIRECT_STREAM_WINDOW: u64 = 1_250_000;

/// A link's mux window: `WINDOW` frames of at most `MAX_CHUNK` bytes
/// (`hide-peer/src/mux/streams.rs`, 16 x 16 KiB).
const MUX_STREAM_WINDOW: u64 = 16 * 16 * 1024;

/// A host's half of `PLACES` streams that writes without end into each:
/// what each write took, partial writes included, and how many streams
/// started and ended.
#[derive(Clone, Default)]
struct Streaming {
    written: Arc<AtomicU64>,
    started: Arc<AtomicUsize>,
    ended: Arc<AtomicUsize>,
}

impl Streaming {
    async fn write_into(&self, mut send: impl AsyncWrite + Unpin) {
        self.started.fetch_add(1, Ordering::SeqCst);
        let chunk = vec![3_u8; 16 * 1024];
        while let Ok(count) = send.write(&chunk).await {
            if count == 0 {
                break;
            }
            self.written.fetch_add(count as u64, Ordering::SeqCst);
        }
        self.ended.fetch_add(1, Ordering::SeqCst);
    }

    async fn all_started(&self) {
        within("every stream to start", async {
            while self.started.load(Ordering::SeqCst) < PLACES {
                tokio::time::sleep(Duration::from_millis(10)).await; // wait-ok: the host's handlers start as the streams arrive
            }
        })
        .await;
    }

    async fn all_ended(&self) {
        within("the host's writes to end", async {
            while self.ended.load(Ordering::SeqCst) < PLACES {
                tokio::time::sleep(Duration::from_millis(10)).await; // wait-ok: the host's writes fail once the connection is closed
            }
        })
        .await;
    }
}

/// Copies each of `streams` into a page through `pipe` on the relay with
/// one shared budget of `limit`, all started together, and checks each
/// ends on the budget, at most one chunk per copy past it; the budget is
/// returned.
async fn on_the_relay<R, W>(streams: Vec<(R, W)>, limit: u64) -> Arc<RelayBudget>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let budget = Arc::new(RelayBudget::new(limit));
    let mut tunnels = Vec::new();
    let mut pages = Vec::new();
    for (recv, send) in streams {
        let (page, local) = tokio::io::duplex(4 * CHUNK);
        pages.push(drain(page));
        let budget = Arc::clone(&budget);
        tunnels.push(tokio::spawn(async move {
            let mut local = local;
            pipe(
                &mut local,
                &[],
                recv,
                send,
                &budget,
                &|| true,
                &slot(),
                &|| {},
            )
            .await
            .0
        }));
    }
    for tunnel in tunnels {
        assert_eq!(within("a tunnel", tunnel).await.unwrap(), Piped::Budget);
    }
    assert!(budget.used() <= limit + 2 * PLACES as u64 * CHUNK as u64);
    budget
}

// R18 inbound: bytes the host sends cross the transport before the guest
// reads them, and the guest counts each read, so a host streaming into
// sixteen tunnels passes what the guest counted by at most what the
// transport lets it send ahead: one stream receive window per tunnel. On
// the real direct transport.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_streaming_into_sixteen_tunnels_passes_the_budget_by_at_most_sixteen_receive_windows()
 {
    let host = Machine::new("windows-pc");
    let guest = Machine::new("macbook");
    let streaming = Streaming::default();
    {
        let streaming = streaming.clone();
        host.host.handle(StreamKind::Page, move |incoming| {
            let streaming = streaming.clone();
            async move {
                let _recv = incoming.stream.recv;
                streaming.write_into(incoming.stream.send).await;
            }
        });
    }
    let link = paired(&host, &guest).await;
    let mut streams = Vec::new();
    for _ in 0..PLACES {
        let stream = open_page(&link).await.unwrap();
        streams.push((stream.recv, stream.send));
    }
    streaming.all_started().await;
    let budget = on_the_relay(streams, 32 * CHUNK as u64).await;
    // The bound holds at every moment: the host's writes are held by the
    // window once the guest stops reading. Closing the connection ends
    // every one of them, so the count read below is the last.
    link.close(hide_peer::protocol::CloseCode::Lost);
    streaming.all_ended().await;
    let past = streaming
        .written
        .load(Ordering::SeqCst)
        .saturating_sub(budget.used());
    assert!(
        past <= PLACES as u64 * DIRECT_STREAM_WINDOW,
        "the host wrote {past} bytes past what the guest counted"
    );
}

// The same bound on a link's mux (the SSH and WSL links), at its own
// window: sixteen streams of a pipe transport, the far end writing without
// end into each.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_streaming_into_sixteen_mux_tunnels_passes_the_budget_by_at_most_sixteen_mux_windows()
 {
    use hide_peer::key::MachineKey;
    use hide_peer::mux::Carrier;
    use hide_peer::pipe::PipeTransport;
    use hide_peer::protocol::{Candidate, PeerAddr};
    use hide_peer::transport::{PeerConn as _, PeerTransport as _};
    use hide_platform::ipc::LocalStream;

    let far = Arc::new(PipeTransport::accepting(MachineKey::generate().unwrap()));
    let near = {
        let far = Arc::clone(&far);
        PipeTransport::dialing(MachineKey::generate().unwrap(), move |_| {
            let (near, other) = LocalStream::pair().unwrap();
            far.arrived(other.into());
            async move { Ok(Carrier::from(near)) }
        })
    };
    let to = PeerAddr::new(far.key(), vec![Candidate::Wsl("Ubuntu".to_owned())]);
    let (dialed, accepted) = within("the connection", async {
        tokio::join!(near.dial(&to), far.accept())
    })
    .await;
    let (ours, theirs) = (dialed.unwrap(), accepted.unwrap());
    let streaming = Streaming::default();
    let mut streams = Vec::new();
    for _ in 0..PLACES {
        let (send, recv) = ours.open(-100).await.unwrap();
        streams.push((recv, send));
        let (their_send, their_recv) = within("the stream", theirs.accept()).await.unwrap();
        let streaming = streaming.clone();
        tokio::spawn(async move {
            let _recv = their_recv;
            streaming.write_into(their_send).await;
        });
    }
    streaming.all_started().await;
    let budget = on_the_relay(streams, 16 * CHUNK as u64).await;
    ours.close(hide_peer::protocol::CloseCode::Lost);
    streaming.all_ended().await;
    let past = streaming
        .written
        .load(Ordering::SeqCst)
        .saturating_sub(budget.used());
    assert!(
        past <= PLACES as u64 * MUX_STREAM_WINDOW,
        "the far end wrote {past} bytes past what the guest counted"
    );
}

// A read that is cancelled (its tunnel ended, its place reclaimed) counts
// nothing: only bytes a read took are counted.
#[tokio::test]
async fn a_cancelled_read_counts_nothing() {
    let budget = Arc::new(RelayBudget::new(4 * CHUNK as u64));
    let (_silent, from) = tokio::io::duplex(CHUNK);
    let (to, _far) = tokio::io::duplex(CHUNK);
    let waiting = {
        let budget = Arc::clone(&budget);
        tokio::spawn(async move {
            let (slot, moved) = (slot(), AtomicU64::new(0));
            copy_reserved(from, to, &[], &budget, &|| true, &slot, &moved, &|| {}).await;
        })
    };
    tokio::task::yield_now().await;
    waiting.abort();
    let _ = waiting.await;
    assert_eq!(budget.used(), 0);
    assert!(!budget.spent());
}

/// The day a test's budget reads, which the test moves.
static TEST_DAY: AtomicU64 = AtomicU64::new(20_000);

fn test_day() -> u64 {
    TEST_DAY.load(Ordering::SeqCst)
}

// A spent day ends with the UTC day: the next one starts from zero, and its
// tunnels carry again.
#[tokio::test]
async fn a_spent_day_carries_again_once_the_day_turns() {
    let budget = RelayBudget::with_clock(CHUNK as u64, test_day);
    budget.add(CHUNK as u64);
    assert!(budget.spent());
    TEST_DAY.fetch_add(1, Ordering::SeqCst);
    assert!(!budget.spent());
    assert_eq!(budget.used(), 0);
    let Ends {
        mut page,
        mut local,
        host,
        near,
    } = ends();
    let (mut host_read, host_write) = tokio::io::split(host);
    let (near_read, near_write) = tokio::io::split(near);
    let tunnel = tokio::spawn(async move {
        pipe(
            &mut local,
            &[],
            near_read,
            near_write,
            &budget,
            &|| true,
            &slot(),
            &|| {},
        )
        .await
        .0
    });
    page.write_all(b"carried").await.unwrap();
    let mut got = [0_u8; 7];
    within("the bytes on the new day", host_read.read_exact(&mut got))
        .await
        .unwrap();
    assert_eq!(&got, b"carried");
    drop((page, host_write, host_read));
    // Both ends closed at once: which one the tunnel saw first does not
    // matter, only that the budget did not end it.
    let ended = within("the tunnel", tunnel).await.unwrap();
    assert_ne!(ended, Piped::Budget);
}
