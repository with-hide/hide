use std::net::{IpAddr, SocketAddr};

pub const HELP: &str = "hide-relay [--listen <IP:port>] [--trusted-proxy <IP>]\n\
Default: --listen 127.0.0.1:8787. GET /health; WebSocket /ws.\n\
Only the named proxy may supply X-Real-IP, which it must overwrite.\n\
No environment variables, configuration files, database, or keys are read.\n";

#[derive(Clone, Copy)]
pub struct Config {
    pub listen: SocketAddr,
    pub trusted_proxy: Option<IpAddr>,
}
impl Config {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, &'static str> {
        let mut args = args.into_iter();
        let mut listen = None;
        let mut trusted_proxy = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => return Ok(None),
                "--listen" if listen.is_none() => {
                    listen = Some(
                        args.next()
                            .ok_or("missing_listen")?
                            .parse()
                            .map_err(|_| "invalid_listen")?,
                    );
                }
                "--trusted-proxy" if trusted_proxy.is_none() => {
                    trusted_proxy = Some(
                        args.next()
                            .ok_or("missing_proxy")?
                            .parse()
                            .map_err(|_| "invalid_proxy")?,
                    );
                }
                _ => return Err("invalid_argument"),
            }
        }
        Ok(Some(Self {
            listen: listen.unwrap_or(SocketAddr::from(([127, 0, 0, 1], 8787))),
            trusted_proxy,
        }))
    }
}
