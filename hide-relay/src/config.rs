use std::{net::SocketAddr, path::PathBuf};

pub const HELP: &str = "hide-relay [--listen <IP:port>] [--trusted-proxy <IP>]\n\
       [--apns-key <path> --apns-key-id <id> --apns-team-id <id> --apns-topic <topic>]\n\
       [--apns-endpoint <url>]\n\
       [--fcm-project <id>] [--fcm-endpoint <url>] [--fcm-metadata-endpoint <url>]\n\
Default: --listen 127.0.0.1:8787. GET /health; WebSocket /ws.\n\
Only the named proxy may supply X-Real-IP, which it must overwrite.\n\
Apple push needs all four --apns-key/--apns-key-id/--apns-team-id/--apns-topic\n\
or none; without them the relay answers push requests unavailable.\n\
--apns-endpoint names a loopback stand-in for Apple in a verification run.\n\
The key file must be owner-only (0400) and is read once at start.\n\
--fcm-project names the Firebase project; the access token comes from the\n\
VM's own service account through the GCE metadata server, with no key file.\n\
Without it the relay answers FCM requests unavailable. --fcm-endpoint and\n\
--fcm-metadata-endpoint name loopback stand-ins for Google in a verification run.\n\
No environment variables, configuration files or database are read.\n";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApnsConfig {
    pub key: PathBuf,
    pub key_id: String,
    pub team_id: String,
    pub topic: String,
    /// A loopback stand-in for Apple; production never sets it.
    pub endpoint: Option<String>,
}

/// Google's FCM: the project, and in a verification run the stand-ins. No
/// secret: the token is the VM's own (`docs/RELAY_PROTOCOL.md`, Push relay).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FcmConfig {
    pub project: String,
    pub endpoints: crate::fcm::FcmEndpoints,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub listen: SocketAddr,
    pub trusted_proxy: Option<std::net::IpAddr>,
    /// All four Apple fields, or none: a relay configured halfway is a
    /// deployment mistake rather than a relay without push.
    pub apns: Option<ApnsConfig>,
    pub fcm: Option<FcmConfig>,
}

impl Config {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, &'static str> {
        let mut args = args.into_iter();
        let mut listen = None;
        let mut trusted_proxy = None;
        let mut key = None;
        let mut key_id = None;
        let mut team_id = None;
        let mut topic = None;
        let mut endpoint = None;
        let mut fcm_project = None;
        let mut fcm_endpoint = None;
        let mut fcm_metadata = None;
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
                "--apns-key" if key.is_none() => {
                    key = Some(PathBuf::from(args.next().ok_or("missing_apns_key")?));
                }
                "--apns-key-id" if key_id.is_none() => {
                    key_id = Some(args.next().ok_or("missing_apns_key_id")?);
                }
                "--apns-team-id" if team_id.is_none() => {
                    team_id = Some(args.next().ok_or("missing_apns_team_id")?);
                }
                "--apns-topic" if topic.is_none() => {
                    topic = Some(args.next().ok_or("missing_apns_topic")?);
                }
                "--apns-endpoint" if endpoint.is_none() => {
                    endpoint = Some(args.next().ok_or("missing_apns_endpoint")?);
                }
                "--fcm-project" if fcm_project.is_none() => {
                    fcm_project = Some(args.next().ok_or("missing_fcm_project")?);
                }
                "--fcm-endpoint" if fcm_endpoint.is_none() => {
                    fcm_endpoint = Some(args.next().ok_or("missing_fcm_endpoint")?);
                }
                "--fcm-metadata-endpoint" if fcm_metadata.is_none() => {
                    fcm_metadata = Some(args.next().ok_or("missing_fcm_metadata_endpoint")?);
                }
                _ => return Err("invalid_argument"),
            }
        }
        // The ids land in headers Apple reads, so a value that could split a
        // header is refused here rather than at the first send.
        for value in [&key_id, &team_id, &topic].into_iter().flatten() {
            if !crate::apns::identifier_valid(value) {
                return Err("invalid_apns_id");
            }
        }
        if let Some(endpoint) = &endpoint
            && !crate::apns::endpoint_is_loopback(endpoint)
        {
            return Err("invalid_apns_endpoint");
        }
        let apns = match (key, key_id, team_id, topic) {
            (None, None, None, None) if endpoint.is_none() => None,
            (Some(key), Some(key_id), Some(team_id), Some(topic)) => Some(ApnsConfig {
                key,
                key_id,
                team_id,
                topic,
                endpoint,
            }),
            _ => return Err("incomplete_apns_config"),
        };
        // The project lands in the path Google reads, and a stand-in may only
        // be this machine, so both are refused here rather than at a send.
        if fcm_project
            .as_deref()
            .is_some_and(|project| !crate::fcm::project_valid(project))
        {
            return Err("invalid_fcm_project");
        }
        if fcm_endpoint
            .as_deref()
            .is_some_and(|endpoint| !crate::apns::endpoint_is_loopback(endpoint))
        {
            return Err("invalid_fcm_endpoint");
        }
        if fcm_metadata
            .as_deref()
            .is_some_and(|endpoint| !crate::fcm::metadata_endpoint_is_loopback(endpoint))
        {
            return Err("invalid_fcm_metadata_endpoint");
        }
        let fcm = match fcm_project {
            Some(project) => Some(FcmConfig {
                project,
                endpoints: crate::fcm::FcmEndpoints {
                    send: fcm_endpoint,
                    metadata: fcm_metadata,
                },
            }),
            None if fcm_endpoint.is_none() && fcm_metadata.is_none() => None,
            None => return Err("incomplete_fcm_config"),
        };
        Ok(Some(Self {
            listen: listen.unwrap_or(SocketAddr::from(([127, 0, 0, 1], 8787))),
            trusted_proxy,
            apns,
            fcm,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apns::endpoint_is_loopback;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn apple_push_is_all_four_fields_or_none() {
        let none = Config::parse(Vec::new()).unwrap().unwrap();
        assert_eq!(none.apns, None);
        let full = Config::parse(args(&[
            "--apns-key",
            "/etc/hide-relay/apns.p8",
            "--apns-key-id",
            "ABC123",
            "--apns-team-id",
            "DEF456",
            "--apns-topic",
            "me.grab.hide.ios",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            full.apns,
            Some(ApnsConfig {
                key: PathBuf::from("/etc/hide-relay/apns.p8"),
                key_id: "ABC123".into(),
                team_id: "DEF456".into(),
                topic: "me.grab.hide.ios".into(),
                endpoint: None,
            })
        );
        for missing in [
            vec![
                "--apns-key",
                "/k",
                "--apns-key-id",
                "A",
                "--apns-team-id",
                "B",
            ],
            vec![
                "--apns-key-id",
                "A",
                "--apns-team-id",
                "B",
                "--apns-topic",
                "t",
            ],
        ] {
            assert_eq!(
                Config::parse(args(&missing)).unwrap_err(),
                "incomplete_apns_config"
            );
        }
    }

    #[test]
    fn fcm_is_a_valid_project_and_stand_ins_only_with_it() {
        use crate::fcm::FcmEndpoints;
        assert_eq!(Config::parse(Vec::new()).unwrap().unwrap().fcm, None);
        let project = Config::parse(args(&["--fcm-project", "hide-android"]))
            .unwrap()
            .unwrap();
        assert_eq!(
            project.fcm,
            Some(FcmConfig {
                project: "hide-android".into(),
                endpoints: FcmEndpoints::default(),
            })
        );
        let stand_ins = Config::parse(args(&[
            "--fcm-project",
            "hide-android",
            "--fcm-endpoint",
            "http://127.0.0.1:8790",
            "--fcm-metadata-endpoint",
            "http://127.0.0.1:8791",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            stand_ins.fcm.unwrap().endpoints,
            FcmEndpoints {
                send: Some("http://127.0.0.1:8790".into()),
                metadata: Some("http://127.0.0.1:8791".into()),
            }
        );
        for (values, error) in [
            (
                vec!["--fcm-endpoint", "http://127.0.0.1:8790"],
                "incomplete_fcm_config",
            ),
            (
                vec!["--fcm-metadata-endpoint", "http://127.0.0.1:8791"],
                "incomplete_fcm_config",
            ),
            (vec!["--fcm-project", "Hide/../x"], "invalid_fcm_project"),
            (
                vec![
                    "--fcm-project",
                    "hide-android",
                    "--fcm-endpoint",
                    "https://fcm.googleapis.com",
                ],
                "invalid_fcm_endpoint",
            ),
            (
                vec![
                    "--fcm-project",
                    "hide-android",
                    "--fcm-metadata-endpoint",
                    "http://169.254.169.254:80",
                ],
                "invalid_fcm_metadata_endpoint",
            ),
            (
                vec![
                    "--fcm-project",
                    "hide-android",
                    "--fcm-metadata-endpoint",
                    "https://127.0.0.1:8791",
                ],
                "invalid_fcm_metadata_endpoint",
            ),
        ] {
            assert_eq!(
                Config::parse(args(&values)).unwrap_err(),
                error,
                "{values:?}"
            );
        }
    }

    #[test]
    fn the_stand_in_endpoint_must_be_on_this_machine() {
        for allowed in [
            "http://127.0.0.1:8788",
            "https://127.0.0.1:8788/",
            "http://127.0.0.1:8788/3/device",
        ] {
            assert!(endpoint_is_loopback(allowed), "{allowed}");
        }
        for refused in [
            "https://api.push.apple.com",
            "http://localhost:8788",
            "http://10.0.0.1:8788",
            "http://127.0.0.1",
            "http://127.0.0.1:0",
        ] {
            assert!(!endpoint_is_loopback(refused), "{refused}");
            assert_eq!(
                Config::parse(args(&[
                    "--apns-key",
                    "/k",
                    "--apns-key-id",
                    "A",
                    "--apns-team-id",
                    "B",
                    "--apns-topic",
                    "t",
                    "--apns-endpoint",
                    refused,
                ]))
                .unwrap_err(),
                "invalid_apns_endpoint"
            );
        }
    }
}
