//! Pure paired-controller v2 wire validation and transcript math, without transport, persistence, or crypto backend.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::Ipv6Addr;

pub const MAX_INVITATION_BYTES: usize = 1024;
pub const MAX_PAIRING_HELLO_BYTES: usize = 512;
pub const MAX_RELAY_FRAME_BYTES: usize = 2 * 1024 * 1024;
pub const ROOM_ID_MIN_BYTES: usize = 22;
pub const ROOM_ID_MAX_BYTES: usize = 128;
pub const MAX_SAFE_SEQUENCE: u64 = (1_u64 << 53) - 1;
pub const DESKTOP_TO_MOBILE_DIRECTION: u8 = 1;
pub const MOBILE_TO_DESKTOP_DIRECTION: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticipantRole {
    Desktop,
    Mobile,
}

impl ParticipantRole {
    fn label(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Mobile => "mobile",
        }
    }

    pub fn send_direction(self) -> u8 {
        match self {
            Self::Desktop => DESKTOP_TO_MOBILE_DIRECTION,
            Self::Mobile => MOBILE_TO_DESKTOP_DIRECTION,
        }
    }

    pub fn receive_direction(self) -> u8 {
        match self {
            Self::Desktop => MOBILE_TO_DESKTOP_DIRECTION,
            Self::Mobile => DESKTOP_TO_MOBILE_DIRECTION,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ControllerInvitationV2 {
    pub version: u8,
    pub relay_url: String,
    pub room: String,
    pub public_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedInvitationV2 {
    pub invitation: ControllerInvitationV2,
    pub public_key: [u8; 65],
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct V2CommitHello {
    #[serde(rename = "type")]
    pub kind: String,
    pub phase: String,
    pub nonce: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct V2RevealHello {
    #[serde(rename = "type")]
    pub kind: String,
    pub phase: String,
    pub nonce: String,
    #[serde(rename = "publicKey")]
    pub public_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SecureEnvelope {
    #[serde(rename = "type")]
    pub kind: String,
    pub seq: u64,
    pub ciphertext: String,
}

pub fn decode_base64url(value: &str) -> Result<Vec<u8>, &'static str> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err("Invalid base64url value.");
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| "Invalid base64url value.")?;
    if URL_SAFE_NO_PAD.encode(&decoded) != value {
        return Err("Non-canonical base64url value.");
    }
    Ok(decoded)
}

pub fn encode_base64url(value: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(value)
}

/// `wss:` is allowed everywhere; `ws:` only for the controller's loopback hosts.
pub fn valid_relay_url(value: &str) -> bool {
    if value.is_empty()
        || value
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace() || b == b'\\')
    {
        return false;
    }
    let Some((scheme, remainder)) = value.split_once("://") else {
        return false;
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "wss" && scheme != "ws" {
        return false;
    }
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.contains('%') {
        return false;
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let Some((address, tail)) = bracketed.split_once(']') else {
            return false;
        };
        let Ok(address) = address.parse::<Ipv6Addr>() else {
            return false;
        };
        let port = if tail.is_empty() {
            None
        } else if let Some(port) = tail.strip_prefix(':') {
            Some(port)
        } else {
            return false;
        };
        (Some((format!("[{address}]"), true)), port)
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) if !port.contains(':') => (host, Some(port)),
            Some(_) => return false,
            None => (authority, None),
        };
        if host.is_empty() || !valid_dns_host(host) {
            return false;
        }
        (Some((host.to_ascii_lowercase(), false)), port)
    };
    if let Some(port) = port {
        if !port.is_empty()
            && (port.len() > 5
                || !port.bytes().all(|b| b.is_ascii_digit())
                || port
                    .parse::<u32>()
                    .map_or(true, |port| port > u16::MAX as u32))
        {
            return false;
        }
    }
    let Some((host, is_ipv6)) = host else {
        return false;
    };
    if scheme == "wss" {
        return true;
    }
    !is_ipv6 && matches!(host.as_str(), "localhost" | "127.0.0.1") || is_ipv6 && host == "[::1]"
}

fn valid_dns_host(host: &str) -> bool {
    if host.is_empty() || !host.is_ascii() {
        return false;
    }
    let host = host.strip_suffix('.').unwrap_or(host);
    !host.is_empty()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

pub fn parse_v2_invitation(json: &str) -> Result<ValidatedInvitationV2, &'static str> {
    if json.len() > MAX_INVITATION_BYTES {
        return Err("Controller invitation is too large.");
    }
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| "Invalid controller invitation.")?;
    let object = value.as_object().ok_or("Invalid controller invitation.")?;
    if object.contains_key("secret") {
        return Err("Version 2 invitations must not contain a shared secret.");
    }
    let invitation: ControllerInvitationV2 =
        serde_json::from_value(value).map_err(|_| "Invalid controller invitation.")?;
    if invitation.version != 2
        || !valid_relay_url(&invitation.relay_url)
        || !(ROOM_ID_MIN_BYTES..=ROOM_ID_MAX_BYTES).contains(&invitation.room.len())
        || !invitation
            .room
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err("Invalid controller invitation.");
    }
    let key = decode_base64url(&invitation.public_key)?;
    let public_key: [u8; 65] = key
        .try_into()
        .map_err(|_| "Invalid controller invitation public key.")?;
    Ok(ValidatedInvitationV2 {
        invitation,
        public_key,
    })
}

pub fn pairing_commitment(
    role: ParticipantRole,
    public_key: &[u8],
    nonce: &[u8],
) -> Result<[u8; 32], &'static str> {
    if public_key.len() != 65 || nonce.len() != 16 {
        return Err("Invalid pairing commitment material.");
    }
    let mut hasher = Sha256::new();
    hasher.update(format!("monitter-controller-v2/commit/{}\0", role.label()).as_bytes());
    hasher.update(public_key);
    hasher.update(nonce);
    Ok(hasher.finalize().into())
}

pub fn verification_code(
    desktop_public: &[u8],
    mobile_public: &[u8],
    desktop_nonce: &[u8],
    mobile_nonce: &[u8],
    shared_secret: &[u8],
) -> Result<String, &'static str> {
    if desktop_public.len() != 65
        || mobile_public.len() != 65
        || desktop_nonce.len() != 16
        || mobile_nonce.len() != 16
        || shared_secret.len() != 32
    {
        return Err("Invalid verification transcript.");
    }
    let mut hasher = Sha256::new();
    hasher.update(desktop_public);
    hasher.update(mobile_public);
    hasher.update(desktop_nonce);
    hasher.update(mobile_nonce);
    hasher.update(shared_secret);
    let digest = hasher.finalize();
    let numeric = u32::from_be_bytes(digest[..4].try_into().expect("digest prefix")) % 1_000_000;
    Ok(format!("{numeric:06}"))
}

pub fn commit_hello(
    role: ParticipantRole,
    public_key: &[u8],
    nonce: &[u8],
) -> Result<V2CommitHello, &'static str> {
    let commitment = pairing_commitment(role, public_key, nonce)?;
    Ok(V2CommitHello {
        kind: "hello".into(),
        phase: "commit".into(),
        nonce: encode_base64url(&commitment),
    })
}

pub fn reveal_hello(public_key: &[u8], nonce: &[u8]) -> Result<V2RevealHello, &'static str> {
    if public_key.len() != 65 || nonce.len() != 16 {
        return Err("Invalid pairing reveal material.");
    }
    Ok(V2RevealHello {
        kind: "hello".into(),
        phase: "reveal".into(),
        nonce: encode_base64url(nonce),
        public_key: encode_base64url(public_key),
    })
}

pub fn parse_commit_hello(json: &str) -> Result<[u8; 32], &'static str> {
    if json.len() > MAX_PAIRING_HELLO_BYTES {
        return Err("Invalid v2 commit envelope.");
    }
    let hello: V2CommitHello =
        serde_json::from_str(json).map_err(|_| "Invalid v2 commit envelope.")?;
    if hello.kind != "hello" || hello.phase != "commit" || hello.nonce.len() > 64 {
        return Err("Invalid v2 commit envelope.");
    }
    decode_base64url(&hello.nonce)?
        .try_into()
        .map_err(|_| "Invalid v2 commitment.")
}

pub fn parse_reveal_hello(json: &str) -> Result<([u8; 65], [u8; 16]), &'static str> {
    if json.len() > MAX_PAIRING_HELLO_BYTES {
        return Err("Invalid v2 reveal envelope.");
    }
    let hello: V2RevealHello =
        serde_json::from_str(json).map_err(|_| "Invalid v2 reveal envelope.")?;
    if hello.kind != "hello"
        || hello.phase != "reveal"
        || hello.nonce.len() > 64
        || hello.public_key.len() > 128
    {
        return Err("Invalid v2 reveal envelope.");
    }
    let public_key = decode_base64url(&hello.public_key)?
        .try_into()
        .map_err(|_| "Invalid reveal public key.")?;
    let nonce = decode_base64url(&hello.nonce)?
        .try_into()
        .map_err(|_| "Invalid reveal nonce.")?;
    Ok((public_key, nonce))
}

pub fn sequence_iv(direction: u8, sequence: u64) -> Result<[u8; 12], &'static str> {
    if !matches!(
        direction,
        DESKTOP_TO_MOBILE_DIRECTION | MOBILE_TO_DESKTOP_DIRECTION
    ) || sequence > MAX_SAFE_SEQUENCE
    {
        return Err("Invalid encrypted message sequence.");
    }
    let mut iv = [0_u8; 12];
    iv[3] = direction;
    iv[4..].copy_from_slice(&sequence.to_be_bytes());
    Ok(iv)
}

pub fn parse_secure_envelope(json: &str) -> Result<SecureEnvelope, &'static str> {
    if json.len() > MAX_RELAY_FRAME_BYTES {
        return Err("Relay frame is too large.");
    }
    let envelope: SecureEnvelope =
        serde_json::from_str(json).map_err(|_| "Invalid secure envelope.")?;
    if envelope.kind != "secure" || envelope.seq > MAX_SAFE_SEQUENCE {
        return Err("Invalid secure envelope.");
    }
    if decode_base64url(&envelope.ciphertext).is_err() {
        return Err("Invalid secure ciphertext.");
    }
    Ok(envelope)
}

/// Advance with `authenticated` only after AES-GCM verifies; failed auth must not consume a sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboundSequence {
    role: ParticipantRole,
    next: u64,
}

impl InboundSequence {
    pub fn new(role: ParticipantRole) -> Self {
        Self { role, next: 0 }
    }

    pub fn iv_for(&self, sequence: u64) -> Result<[u8; 12], &'static str> {
        if sequence != self.next {
            return Err("Rejected replayed or out-of-order encrypted message.");
        }
        sequence_iv(self.role.receive_direction(), sequence)
    }

    pub fn authenticated(&mut self, sequence: u64) -> Result<(), &'static str> {
        if sequence != self.next {
            return Err("Rejected replayed or out-of-order encrypted message.");
        }
        if sequence > MAX_SAFE_SEQUENCE {
            return Err("Invalid encrypted message sequence.");
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or("Invalid encrypted message sequence.")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> [u8; 65] {
        let mut value = [byte; 65];
        value[0] = 4;
        value
    }

    #[test]
    fn validates_v2_invitation_limits_and_supported_relay_schemes() {
        let public_key = encode_base64url(&key(0x11));
        let invitation = format!(
            r#"{{"version":2,"relayUrl":"wss://relay.example/relay","room":"abcdefghijklmnopqrstuv","publicKey":"{public_key}"}}"#
        );
        let parsed = parse_v2_invitation(&invitation).unwrap();
        assert_eq!(parsed.invitation.version, 2);
        assert_eq!(parsed.public_key, key(0x11));

        for url in [
            "ws://localhost:8787/relay",
            "ws://127.0.0.1:8787",
            "ws://[::1]:8787",
        ] {
            assert!(valid_relay_url(url), "{url}");
        }
        for url in [
            "http://relay.example",
            "ws://relay.example",
            "ws://127.0.0.2",
            "wss://user@relay.example",
        ] {
            assert!(!valid_relay_url(url), "{url}");
        }
        let too_large = " ".repeat(MAX_INVITATION_BYTES + 1);
        assert_eq!(
            parse_v2_invitation(&too_large),
            Err("Controller invitation is too large.")
        );
        let with_secret = invitation.replace(
            "\"version\":2",
            "\"version\":2,\"secret\":\"must-not-be-present\"",
        );
        assert!(parse_v2_invitation(&with_secret).is_err());
    }

    #[test]
    fn v2_commit_reveal_envelopes_match_wire_names_and_fixed_commitment_vector() {
        let public = key(0x11);
        let nonce = [0x22; 16];
        let commitment = pairing_commitment(ParticipantRole::Desktop, &public, &nonce).unwrap();
        assert_eq!(
            commitment,
            [
                0xaa, 0xea, 0x04, 0x87, 0xb9, 0xd6, 0x57, 0x84, 0xec, 0x8c, 0x82, 0x50, 0x24, 0x99,
                0xd4, 0x40, 0xdf, 0x7f, 0x20, 0x5f, 0xdc, 0x8f, 0x1d, 0x78, 0x45, 0x50, 0xfd, 0x04,
                0x5a, 0xb5, 0x08, 0x6a,
            ]
        );
        let commit = commit_hello(ParticipantRole::Desktop, &public, &nonce).unwrap();
        let commit_json = serde_json::to_string(&commit).unwrap();
        assert_eq!(
            commit_json,
            r#"{"type":"hello","phase":"commit","nonce":"quoEh7nWV4TsjIJQJJnUQN9_IF_cjx14RVD9BFq1CGo"}"#
        );
        assert_eq!(parse_commit_hello(&commit_json).unwrap(), commitment);

        let reveal = reveal_hello(&public, &nonce).unwrap();
        let reveal_json = serde_json::to_string(&reveal).unwrap();
        assert_eq!(
            reveal_json,
            r#"{"type":"hello","phase":"reveal","nonce":"IiIiIiIiIiIiIiIiIiIiIg","publicKey":"BBERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERERE"}"#
        );
        assert_eq!(parse_reveal_hello(&reveal_json).unwrap(), (public, nonce));
        assert!(parse_reveal_hello(
            r#"{"type":"hello","phase":"reveal","nonce":"AA==","publicKey":"AA"}"#
        )
        .is_err());
    }

    #[test]
    fn verification_code_and_directional_sequence_iv_follow_controller_bytes() {
        let desktop_public = key(0x11);
        let mobile_public = key(0x33);
        let desktop_nonce = [0x44; 16];
        let mobile_nonce = [0x55; 16];
        let shared = [0x66; 32];
        assert_eq!(
            verification_code(
                &desktop_public,
                &mobile_public,
                &desktop_nonce,
                &mobile_nonce,
                &shared
            )
            .unwrap(),
            "763416"
        );
        let desktop_iv = sequence_iv(ParticipantRole::Desktop.send_direction(), 0x0102).unwrap();
        assert_eq!(desktop_iv[3], DESKTOP_TO_MOBILE_DIRECTION);
        assert_eq!(&desktop_iv[4..], &0x0102_u64.to_be_bytes());
        let mobile_iv = sequence_iv(ParticipantRole::Mobile.send_direction(), 0x0102).unwrap();
        assert_eq!(mobile_iv[3], MOBILE_TO_DESKTOP_DIRECTION);
        assert_ne!(desktop_iv, mobile_iv);
        assert!(sequence_iv(3, 0).is_err());
        assert!(sequence_iv(DESKTOP_TO_MOBILE_DIRECTION, MAX_SAFE_SEQUENCE + 1).is_err());

        let mut receive = InboundSequence::new(ParticipantRole::Desktop);
        assert_eq!(receive.iv_for(0).unwrap()[3], MOBILE_TO_DESKTOP_DIRECTION);
        assert!(receive.iv_for(1).is_err());
        receive.authenticated(0).unwrap();
        assert!(receive.iv_for(0).is_err());
        assert_eq!(receive.iv_for(1).unwrap()[3], MOBILE_TO_DESKTOP_DIRECTION);
    }

    #[test]
    fn secure_envelope_parser_enforces_type_size_sequence_and_canonical_bytes() {
        let valid = r#"{"type":"secure","seq":0,"ciphertext":"AQID"}"#;
        assert_eq!(parse_secure_envelope(valid).unwrap().seq, 0);
        assert!(parse_secure_envelope(
            r#"{"type":"secure","seq":9007199254740992,"ciphertext":"AQID"}"#
        )
        .is_err());
        assert!(parse_secure_envelope(r#"{"type":"secure","seq":0,"ciphertext":"AQI="}"#).is_err());
        assert!(
            parse_secure_envelope(r#"{"type":"response","seq":0,"ciphertext":"AQID"}"#).is_err()
        );
        assert!(parse_secure_envelope(&"x".repeat(MAX_RELAY_FRAME_BYTES + 1)).is_err());
    }
}
