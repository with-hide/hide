//! Paired phones and the pairing code (PRD D-04, D-19).
//!
//! One pairing code lives at a time, in memory only: five minutes, one use,
//! and a new code voids the old one. A paired phone holds a credential of its
//! own; `phones.json` keeps only its SHA-256, the phone's name, when it was
//! last seen, its push subscription and the address it was paired at. At most
//! four phones per address; a phone not seen for seven days is revoked, and a
//! revoke removes the credential and the subscription in one write.
//!
//! Only the phones paired at the address this core was last exposed at count
//! (PRD core-host-node-move B17): they are listed, belled and let in, and the
//! limit counts them. A phone paired where the core ran before it moved waits,
//! unlisted, for a move back, and the seven-day rule ends it otherwise.

use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use subtle::ConstantTimeEq;

use super::store::{self, Notifications, PhoneOrigin, PhoneRecord, PhonesFile, PushSubscription};

pub const MAX_PHONES: usize = 4;
pub const CODE_TTL_MS: u64 = 5 * 60 * 1000;
pub const INACTIVE_REVOKE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PairRefusal {
    /// Expired, already used, replaced by a newer code, or never issued.
    CodeExpired,
    /// Four phones are paired already at this address.
    PhoneLimit,
}

impl PairRefusal {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::CodeExpired => "code_expired",
            Self::PhoneLimit => "phone_limit",
        }
    }
}

struct PairingCode {
    code: String,
    expires_at_ms: u64,
}

pub struct Phones {
    path: PathBuf,
    file: PhonesFile,
    code: Option<PairingCode>,
    /// The tailnet name this core was last exposed at, which a switch-off or a
    /// failed check leaves as it is; none before its first exposure on
    /// record.
    origin: Option<PhoneOrigin>,
}

/// Why a credential lets no phone in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unadmitted {
    /// No phone holds it.
    Unknown,
    /// Its phone was paired at another address than this core's.
    Elsewhere,
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0_u8; N];
    getrandom::getrandom(&mut bytes).expect("getrandom");
    bytes
}

pub fn credential_hash(credential: &str) -> String {
    hex::encode(ring::digest::digest(
        &ring::digest::SHA256,
        credential.as_bytes(),
    ))
}

/// A readable name from the phone's user agent; the PWA cannot read a model.
pub fn name_from_user_agent(user_agent: &str) -> &'static str {
    if user_agent.contains("iPad") {
        "iPad"
    } else if user_agent.contains("iPhone") {
        "iPhone"
    } else if user_agent.contains("Android") {
        "Android"
    } else {
        "Phone"
    }
}

impl Phones {
    /// The phones on file; one paired before phones kept their address
    /// gets `recorded`, the exposure the settings name, when there is one.
    pub fn load(path: PathBuf, recorded: Option<&str>) -> Self {
        let file: PhonesFile = store::read(&path);
        let mut phones = Self {
            path,
            file,
            code: None,
            origin: None,
        };
        if let Some(recorded) = recorded {
            phones.set_origin(recorded);
        }
        phones
    }

    fn stamp(&mut self, origin: &str) {
        let mut stamped = false;
        for phone in &mut self.file.phones {
            if phone.origin == PhoneOrigin::Unrecorded {
                phone.origin = PhoneOrigin::Paired(origin.to_owned());
                stamped = true;
            }
        }
        if stamped {
            self.save();
        }
    }

    /// The address the core is exposed at now.
    pub fn set_origin(&mut self, origin: &str) {
        self.stamp(origin);
        self.origin = Some(PhoneOrigin::Paired(origin.to_owned()));
    }

    pub fn set_scope(&mut self, scope: Option<PhoneOrigin>) {
        self.clear_code();
        self.origin = scope;
    }

    fn here(&self, phone: &PhoneRecord) -> bool {
        self.origin.as_ref() == Some(&phone.origin)
    }

    /// The phones paired at the address this core was last exposed at.
    pub fn current(&self) -> impl Iterator<Item = &PhoneRecord> {
        self.file.phones.iter().filter(|phone| self.here(phone))
    }

    fn save(&self) -> bool {
        store::write_logged(&self.path, &self.file)
    }

    pub fn is_empty(&self) -> bool {
        self.file.phones.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&PhoneRecord> {
        self.file.phones.iter().find(|phone| phone.id == id)
    }

    /// A fresh code, voiding any earlier one.
    pub fn new_code(&mut self, now_ms: u64) -> (String, u64) {
        let code = URL_SAFE_NO_PAD.encode(random_bytes::<16>());
        let expires_at_ms = now_ms + CODE_TTL_MS;
        self.code = Some(PairingCode {
            code: code.clone(),
            expires_at_ms,
        });
        (code, expires_at_ms)
    }

    /// The live code and when it expires, if one was issued and is unused.
    pub fn code(&self) -> Option<(&str, u64)> {
        self.code
            .as_ref()
            .map(|code| (code.code.as_str(), code.expires_at_ms))
    }

    pub fn clear_code(&mut self) {
        self.code = None;
    }

    /// Pairs a phone with the live code. The code is spent only by a pairing
    /// that succeeds; the phone limit leaves it for after a revoke. Two
    /// phones racing on one code are serialized by the caller's lock, so the
    /// second finds it spent.
    pub fn pair(
        &mut self,
        offered: &str,
        user_agent: &str,
        now_ms: u64,
    ) -> Result<(PhoneRecord, String), PairRefusal> {
        let valid = self.code.as_ref().is_some_and(|code| {
            now_ms < code.expires_at_ms
                && code.code.len() == offered.len()
                && bool::from(code.code.as_bytes().ct_eq(offered.as_bytes()))
        });
        let Some(origin) = self.origin.clone().filter(|_| valid) else {
            return Err(PairRefusal::CodeExpired);
        };
        if self.current().count() >= MAX_PHONES {
            return Err(PairRefusal::PhoneLimit);
        }
        self.code = None;
        let credential = hex::encode(random_bytes::<32>());
        let base = name_from_user_agent(user_agent);
        let taken = |name: &str| self.current().any(|phone| phone.name == name);
        let name = if taken(base) {
            (2..)
                .map(|n| format!("{base} {n}"))
                .find(|name| !taken(name))
                .expect("an unused name")
        } else {
            base.to_owned()
        };
        let record = PhoneRecord {
            id: hex::encode(random_bytes::<8>()),
            name,
            credential_sha256: credential_hash(&credential),
            paired_at_ms: now_ms,
            last_seen_ms: now_ms,
            notifications: Notifications::Unasked,
            push: None,
            origin,
        };
        self.file.phones.push(record.clone());
        self.save();
        Ok((record, credential))
    }

    /// The phone a credential belongs to, when it was paired at the address
    /// this core was last exposed at. Every record is compared, in constant time per record, so
    /// the answer's timing does not say which.
    pub fn authenticate(&self, credential: &str) -> Result<&PhoneRecord, Unadmitted> {
        let offered = credential_hash(credential);
        let mut found = None;
        for phone in &self.file.phones {
            if bool::from(phone.credential_sha256.as_bytes().ct_eq(offered.as_bytes())) {
                found = Some(phone);
            }
        }
        let phone = found.ok_or(Unadmitted::Unknown)?;
        if self.here(phone) {
            Ok(phone)
        } else {
            Err(Unadmitted::Elsewhere)
        }
    }

    pub fn touch(&mut self, id: &str, now_ms: u64) {
        if let Some(phone) = self.file.phones.iter_mut().find(|phone| phone.id == id) {
            phone.last_seen_ms = now_ms;
            self.save();
        }
    }

    /// Revokes one phone; `None` when it was already gone, so a second
    /// revoke is a quiet no-op.
    pub fn revoke(&mut self, id: &str) -> Option<PhoneRecord> {
        let index = self.file.phones.iter().position(|phone| phone.id == id)?;
        let removed = self.file.phones.remove(index);
        self.save();
        Some(removed)
    }

    /// Revokes every phone not seen for seven days, except the ones in
    /// `connected` (a phone on a live connection is being seen now).
    pub fn sweep(&mut self, now_ms: u64, connected: &[String]) -> Vec<PhoneRecord> {
        let (stale, kept): (Vec<_>, Vec<_>) = self.file.phones.drain(..).partition(|phone| {
            !connected.contains(&phone.id)
                && now_ms.saturating_sub(phone.last_seen_ms) >= INACTIVE_REVOKE_MS
        });
        self.file.phones = kept;
        if !stale.is_empty() {
            self.save();
        }
        stale
    }

    pub fn set_subscription(&mut self, id: &str, push: Option<PushSubscription>) -> bool {
        let Some(phone) = self.file.phones.iter_mut().find(|phone| phone.id == id) else {
            return false;
        };
        let notifications = if push.is_some() {
            Notifications::On
        } else {
            phone.notifications
        };
        if phone.push == push && phone.notifications == notifications {
            return false;
        }
        phone.push = push;
        phone.notifications = notifications;
        self.save();
        true
    }

    pub fn set_notifications(&mut self, id: &str, notifications: Notifications) -> bool {
        let Some(phone) = self.file.phones.iter_mut().find(|phone| phone.id == id) else {
            return false;
        };
        let push = if notifications == Notifications::Off {
            None
        } else {
            phone.push.clone()
        };
        if phone.notifications == notifications && phone.push == push {
            return false;
        }
        phone.notifications = notifications;
        phone.push = push;
        self.save();
        true
    }

    /// Drops a subscription the push service called gone; the phone
    /// registers a new one on its next connection.
    pub fn drop_subscription(&mut self, id: &str, endpoint: &str) -> bool {
        let Some(phone) = self.file.phones.iter_mut().find(|phone| {
            phone.id == id
                && phone
                    .push
                    .as_ref()
                    .is_some_and(|push| push.endpoint == endpoint)
        }) else {
            return false;
        };
        phone.push = None;
        self.save();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)";

    const MAC: &str = "mac.tailnet.ts.net";

    fn phones() -> (tempfile::TempDir, Phones) {
        let dir = tempfile::tempdir().unwrap();
        let phones = Phones::load(store::phones_path(dir.path()), Some(MAC));
        (dir, phones)
    }

    fn ids(phones: &Phones) -> Vec<String> {
        phones.current().map(|phone| phone.id.clone()).collect()
    }

    fn pair_one(phones: &mut Phones, now: u64) -> (PhoneRecord, String) {
        let (code, _) = phones.new_code(now);
        phones.pair(&code, UA, now).unwrap()
    }

    /// A phone paired before phones kept their address takes the exposure
    /// the settings name at the first load, and is written with it
    /// (amendment 12).
    #[test]
    fn a_phone_paired_before_phones_kept_their_address_takes_the_one_on_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = store::phones_path(dir.path());
        let credential = "c".repeat(64);
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({"phones": [{
                "id": "p1", "name": "iPhone", "credential_sha256": credential_hash(&credential),
                "paired_at_ms": 1, "last_seen_ms": 1,
            }]}))
            .unwrap(),
        )
        .unwrap();
        let phones = Phones::load(path.clone(), Some(MAC));
        assert_eq!(
            phones.authenticate(&credential).map(|phone| &phone.id),
            Ok(&"p1".to_owned())
        );
        let reread: PhonesFile = store::read(&path);
        assert_eq!(reread.phones[0].origin, PhoneOrigin::Paired(MAC.to_owned()));
    }

    /// After the core moves, the phones paired where it ran wait unlisted
    /// and do not fill the new address's four; a move back lets them in
    /// again (B17).
    #[test]
    fn phones_paired_where_the_core_ran_before_wait_for_it_and_leave_the_limit_alone() {
        let (_dir, mut phones) = phones();
        let (first, credential) = pair_one(&mut phones, 1);
        for _ in 1..MAX_PHONES {
            pair_one(&mut phones, 1);
        }
        phones.set_origin("mini.tailnet.ts.net");
        assert!(ids(&phones).is_empty());
        assert_eq!(
            phones.authenticate(&credential).unwrap_err(),
            Unadmitted::Elsewhere
        );
        for _ in 0..MAX_PHONES {
            pair_one(&mut phones, 2);
        }
        let (code, _) = phones.new_code(2);
        assert_eq!(
            phones.pair(&code, UA, 2).unwrap_err(),
            PairRefusal::PhoneLimit
        );
        phones.set_origin(MAC);
        assert_eq!(ids(&phones).len(), MAX_PHONES);
        assert_eq!(
            phones.authenticate(&credential).map(|phone| &phone.id),
            Ok(&first.id)
        );
    }

    /// What is kept is bounded by four phones per address, and the
    /// seven-day rule ends the phones of an address the core left (B17).
    #[test]
    fn the_phones_kept_are_four_per_address_until_the_seven_day_rule() {
        let (_dir, mut phones) = phones();
        let day = 24 * 60 * 60 * 1000;
        for at in 0..5_u64 {
            phones.set_origin(&format!("machine-{at}.tailnet.ts.net"));
            for _ in 0..=MAX_PHONES {
                let (code, _) = phones.new_code(at * day);
                let _ = phones.pair(&code, UA, at * day);
            }
        }
        assert_eq!(phones.file.phones.len(), 5 * MAX_PHONES);
        phones.sweep(8 * day, &[]);
        assert_eq!(phones.file.phones.len(), 3 * MAX_PHONES);
    }

    #[test]
    fn a_code_pairs_once_and_a_new_code_voids_the_old() {
        let (_dir, mut phones) = phones();
        let (old, _) = phones.new_code(1_000);
        let (code, expires) = phones.new_code(2_000);
        assert_eq!(expires, 2_000 + CODE_TTL_MS);
        assert_eq!(
            phones.pair(&old, UA, 3_000).unwrap_err(),
            PairRefusal::CodeExpired
        );
        let (record, credential) = phones.pair(&code, UA, 3_000).unwrap();
        assert_eq!(record.name, "iPhone");
        // The second phone on the same code finds it spent.
        assert_eq!(
            phones.pair(&code, UA, 3_001).unwrap_err(),
            PairRefusal::CodeExpired
        );
        assert_eq!(
            phones
                .authenticate(&credential)
                .map(|phone| phone.id.clone()),
            Ok(record.id.clone())
        );
        assert!(phones.authenticate("0".repeat(64).as_str()).is_err());
        // Only the hash is on disk.
        let text = std::fs::read_to_string(&phones.path).unwrap();
        assert!(!text.contains(&credential));
        assert!(!text.contains(&code));
    }

    #[test]
    fn a_code_expires_after_five_minutes() {
        let (_dir, mut phones) = phones();
        let (code, expires) = phones.new_code(0);
        assert_eq!(
            phones.pair(&code, UA, expires).unwrap_err(),
            PairRefusal::CodeExpired
        );
        let (code, _) = phones.new_code(0);
        assert!(phones.pair(&code, UA, CODE_TTL_MS - 1).is_ok());
    }

    #[test]
    fn a_fifth_phone_is_refused_and_keeps_the_code() {
        let (_dir, mut phones) = phones();
        for _ in 0..MAX_PHONES {
            let (code, _) = phones.new_code(0);
            phones.pair(&code, UA, 1).unwrap();
        }
        let names: Vec<_> = phones.current().map(|phone| phone.name.clone()).collect();
        assert_eq!(names, ["iPhone", "iPhone 2", "iPhone 3", "iPhone 4"]);
        let (code, _) = phones.new_code(0);
        assert_eq!(
            phones.pair(&code, UA, 1).unwrap_err(),
            PairRefusal::PhoneLimit
        );
        let first = ids(&phones)[0].clone();
        phones.revoke(&first);
        assert!(
            phones.pair(&code, UA, 2).is_ok(),
            "the code survives the refusal"
        );
    }

    #[test]
    fn revoke_is_idempotent_and_takes_the_subscription_with_it() {
        let (dir, mut phones) = phones();
        let (code, _) = phones.new_code(0);
        let (record, credential) = phones.pair(&code, UA, 1).unwrap();
        phones.set_subscription(
            &record.id,
            Some(PushSubscription {
                endpoint: "https://web.push.apple.com/x".into(),
                p256dh: "k".into(),
                auth: "a".into(),
            }),
        );
        assert!(phones.revoke(&record.id).is_some());
        assert!(phones.revoke(&record.id).is_none());
        assert!(phones.authenticate(&credential).is_err());
        let reread = Phones::load(store::phones_path(dir.path()), Some(MAC));
        assert!(reread.is_empty());
        let text = std::fs::read_to_string(store::phones_path(dir.path())).unwrap();
        assert!(!text.contains("web.push.apple.com"));
    }

    #[test]
    fn seven_days_unseen_revokes_unless_connected() {
        let (_dir, mut phones) = phones();
        let (code, _) = phones.new_code(0);
        let (old, _) = phones.pair(&code, UA, 0).unwrap();
        let (code, _) = phones.new_code(0);
        let (live, _) = phones.pair(&code, UA, 0).unwrap();
        let (code, _) = phones.new_code(0);
        let (recent, _) = phones.pair(&code, UA, 0).unwrap();
        phones.touch(&recent.id, INACTIVE_REVOKE_MS - 10);
        let revoked = phones.sweep(INACTIVE_REVOKE_MS, std::slice::from_ref(&live.id));
        assert_eq!(
            revoked.iter().map(|phone| &phone.id).collect::<Vec<_>>(),
            [&old.id]
        );
        let left = ids(&phones);
        assert_eq!(left, [live.id, recent.id]);
    }
}
