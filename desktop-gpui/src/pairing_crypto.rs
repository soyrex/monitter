//! Native implementation of controller/secure-session.ts, with no browser runtime.
//! Identity serialization contains a private scalar: persist only in protected storage.
//! Browser IndexedDB CryptoKeys are nonextractable and cannot be migrated here.
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::{PublicKey, SecretKey, elliptic_curve::sec1::ToSec1Point};
use ring::{aead, digest, hkdf, rand::{SecureRandom, SystemRandom}};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

pub type Result<T> = std::result::Result<T, String>;
const MAX_SAFE_SEQUENCE: u64 = 9_007_199_254_740_991;
const MAX_PLAINTEXT_BYTES: usize = 32 * 1024 * 1024;

pub fn bytes_to_base64url(bytes: &[u8]) -> String { URL_SAFE_NO_PAD.encode(bytes) }
pub fn base64url_to_bytes(value: &str) -> Result<Vec<u8>> {
    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') {
        return Err("Invalid base64url value.".into());
    }
    URL_SAFE_NO_PAD.decode(value).map_err(|_| "Invalid base64url value.".into())
}
pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    SystemRandom::new().fill(&mut bytes).map_err(|_| "Secure randomness unavailable.".to_owned())?;
    Ok(bytes)
}

/// Reusable P-256 identity, also usable as a fresh ephemeral handshake identity.
/// Deliberately has no Debug implementation.
#[derive(Clone)]
pub struct PairingIdentity { secret: SecretKey }
impl PairingIdentity {
    pub fn generate() -> Result<Self> {
        loop {
            let mut scalar = random_bytes::<32>()?;
            let secret = SecretKey::from_slice(&scalar);
            scalar.fill(0);
            if let Ok(secret) = secret { return Ok(Self { secret }); }
        }
    }
    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.secret.public_key().to_sec1_point(false).as_bytes().to_vec()
    }
    pub fn public_key(&self) -> Result<String> { Ok(bytes_to_base64url(&self.public_key_bytes())) }
    pub fn derive_secret(&self, peer: &[u8]) -> Result<[u8; 32]> {
        if peer.len() != 65 || peer[0] != 4 { return Err("Invalid P-256 public key.".into()); }
        let peer = PublicKey::from_sec1_bytes(peer).map_err(|_| "Invalid P-256 public key.".to_owned())?;
        let shared = self.secret.diffie_hellman(&peer);
        let mut bytes = [0; 32];
        bytes.copy_from_slice(shared.raw_secret_bytes());
        Ok(bytes)
    }
}
impl Serialize for PairingIdentity {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut record = serializer.serialize_struct("PairingIdentity", 1)?;
        let mut scalar = self.secret.to_bytes();
        let encoded = bytes_to_base64url(scalar.as_slice());
        scalar.fill(0);
        record.serialize_field("privateKey", &encoded)?;
        record.end()
    }
}
impl<'de> Deserialize<'de> for PairingIdentity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Stored { #[serde(rename = "privateKey")] private_key: String }
        let stored = Stored::deserialize(deserializer)?;
        if stored.private_key.len() != 43 { return Err(serde::de::Error::custom("Invalid native pairing identity.")); }
        let mut bytes = base64url_to_bytes(&stored.private_key).map_err(serde::de::Error::custom)?;
        let secret = if bytes.len() == 32 { SecretKey::from_slice(&bytes).ok() } else { None };
        bytes.fill(0);
        secret.map(|secret| Self { secret }).ok_or_else(|| serde::de::Error::custom("Invalid native pairing identity."))
    }
}

pub fn pairing_commitment(role: &str, public: &[u8], nonce: &[u8]) -> Result<[u8; 32]> {
    if !matches!(role, "desktop" | "mobile") || public.len() != 65 || nonce.len() != 16 {
        return Err("Invalid pairing commitment material.".into());
    }
    let mut hash = digest::Context::new(&digest::SHA256);
    hash.update(format!("monitter-controller-v2/commit/{role}\0").as_bytes());
    hash.update(public); hash.update(nonce);
    let mut result = [0; 32]; result.copy_from_slice(hash.finish().as_ref()); Ok(result)
}
pub fn verification_code(desktop_public: &[u8], mobile_public: &[u8], desktop_nonce: &[u8], mobile_nonce: &[u8], secret: &[u8]) -> Result<String> {
    if desktop_public.len() != 65 || mobile_public.len() != 65 || desktop_nonce.len() != 16 || mobile_nonce.len() != 16 || secret.len() != 32 {
        return Err("Invalid verification transcript.".into());
    }
    let mut hash = digest::Context::new(&digest::SHA256);
    for bytes in [desktop_public, mobile_public, desktop_nonce, mobile_nonce, secret] { hash.update(bytes); }
    let hash = hash.finish();
    let number = u32::from_be_bytes(hash.as_ref()[..4].try_into().map_err(|_| "Invalid digest.")?) % 1_000_000;
    Ok(format!("{number:06}"))
}
fn same_bytes(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && left.iter().zip(right).fold(0u8, |difference, (a,b)| difference | (a ^ b)) == 0
}
struct KeyLength;
impl hkdf::KeyType for KeyLength { fn len(&self) -> usize { 32 } }
fn directional_key(secret: &[u8], salt: &[u8], protocol: u8, direction: &str) -> Result<aead::LessSafeKey> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(secret);
    let label = format!("monitter-controller-v{protocol}/{direction}");
    let info = [label.as_bytes()];
    let okm = prk.expand(&info, KeyLength).map_err(|_| "Cannot derive pairing key.".to_owned())?;
    let mut bytes = [0; 32];
    okm.fill(&mut bytes).map_err(|_| "Cannot derive pairing key.".to_owned())?;
    let key = aead::UnboundKey::new(&aead::AES_256_GCM, &bytes);
    bytes.fill(0);
    key.map(aead::LessSafeKey::new).map_err(|_| "Cannot initialize pairing cipher.".into())
}
fn sequence_nonce(direction: u8, sequence: u64) -> Result<aead::Nonce> {
    if sequence > MAX_SAFE_SEQUENCE { return Err("Encrypted message sequence exhausted.".into()); }
    let mut nonce = [0; 12]; nonce[3] = direction; nonce[4..].copy_from_slice(&sequence.to_be_bytes());
    Ok(aead::Nonce::assume_unique_for_key(nonce))
}

pub struct SecureChannel {
    send: aead::LessSafeKey, receive: aead::LessSafeKey,
    send_direction: u8, receive_direction: u8, outbound: u64, inbound: u64,
}
impl SecureChannel {
    /// Matches deriveDirectionalKeys; supports legacy v1 material and current v2 ECDH.
    pub fn new(secret: &[u8], desktop_nonce: &[u8], mobile_nonce: &[u8], desktop: bool, protocol: u8) -> Result<Self> {
        if secret.len() != 32 || desktop_nonce.len() != 16 || mobile_nonce.len() != 16 || !matches!(protocol, 1 | 2) {
            return Err("Invalid secure-session key material.".into());
        }
        let mut salt = [0;32]; salt[..16].copy_from_slice(desktop_nonce); salt[16..].copy_from_slice(mobile_nonce);
        let d2m = directional_key(secret, &salt, protocol, "desktop-to-mobile")?;
        let m2d = directional_key(secret, &salt, protocol, "mobile-to-desktop")?;
        let (send, receive, send_direction, receive_direction) = if desktop { (d2m,m2d,1,2) } else { (m2d,d2m,2,1) };
        Ok(Self { send, receive, send_direction, receive_direction, outbound:0, inbound:0 })
    }
    pub fn seal(&mut self, value: &Value) -> Result<Value> {
        let mut bytes = serde_json::to_vec(value).map_err(|_| "Invalid encrypted JSON.".to_owned())?;
        if bytes.len() > MAX_PLAINTEXT_BYTES { return Err("Encrypted message exceeds size limit.".into()); }
        let seq = self.outbound;
        let nonce = sequence_nonce(self.send_direction, seq)?;
        self.outbound += 1;
        self.send.seal_in_place_append_tag(nonce, aead::Aad::empty(), &mut bytes).map_err(|_| "Cannot seal pairing message.".to_owned())?;
        Ok(json!({"type":"secure", "seq":seq, "ciphertext":bytes_to_base64url(&bytes)}))
    }
    pub fn open(&mut self, envelope: &Value) -> Result<Value> {
        let seq = envelope.get("seq").and_then(Value::as_u64).ok_or("Invalid encrypted message sequence.")?;
        if envelope.get("type").and_then(Value::as_str) != Some("secure") || seq != self.inbound {
            return Err("Rejected replayed or out-of-order encrypted message.".into());
        }
        let encoded = envelope.get("ciphertext").and_then(Value::as_str).ok_or("Invalid encrypted ciphertext.")?;
        if encoded.len() > (MAX_PLAINTEXT_BYTES + 16).div_ceil(3) * 4 { return Err("Encrypted message exceeds size limit.".into()); }
        let mut bytes = base64url_to_bytes(encoded)?;
        let plaintext = self.receive.open_in_place(sequence_nonce(self.receive_direction,seq)?, aead::Aad::empty(), &mut bytes)
            .map_err(|_| "Pairing message authentication failed.".to_owned())?;
        self.inbound += 1;
        serde_json::from_slice(plaintext).map_err(|_| "Invalid encrypted JSON.".into())
    }
}

/// One desktop-side v2 connection. Drop on disconnect; never replay queued mutations.
/// A completed handshake proves key possession, not user approval or owner access.
pub struct CryptoSession {
    identity: PairingIdentity, nonce: [u8;16], public: Vec<u8>,
    commitment_sent: bool, peer_commitment: Option<[u8;32]>, peer_public: Option<String>,
    verification: Option<String>, channel: Option<SecureChannel>, failed: bool,
}
impl CryptoSession {
    pub fn new(identity: &PairingIdentity) -> Result<Self> {
        Ok(Self { identity:identity.clone(), nonce:random_bytes()?, public:identity.public_key_bytes(),
            commitment_sent:false, peer_commitment:None, peer_public:None, verification:None, channel:None, failed:false })
    }
    pub fn hello(&mut self) -> Result<Value> {
        if self.failed || self.commitment_sent { return Err("Pairing commitment already sent or session closed.".into()); }
        let commitment = pairing_commitment("desktop", &self.public, &self.nonce)?;
        self.commitment_sent = true;
        Ok(json!({"type":"hello","phase":"commit","nonce":bytes_to_base64url(&commitment)}))
    }
    pub fn receive_hello(&mut self, value: &Value) -> Result<Option<Value>> {
        if self.failed { return Err("Pairing session closed.".into()); }
        let result = self.receive_hello_inner(value);
        if result.is_err() { self.failed = true; self.channel = None; }
        result
    }
    fn receive_hello_inner(&mut self, value: &Value) -> Result<Option<Value>> {
        if value["type"] != "hello" || !self.commitment_sent || self.channel.is_some() { return Err("Unexpected pairing hello.".into()); }
        let nonce_text = value.get("nonce").and_then(Value::as_str).filter(|v|v.len()<=64).ok_or("Invalid pairing nonce.")?;
        let nonce = base64url_to_bytes(nonce_text)?;
        match value.get("phase").and_then(Value::as_str) {
            Some("commit") => {
                if self.peer_commitment.is_some() || nonce.len()!=32 { return Err("Unexpected pairing commitment.".into()); }
                self.peer_commitment = Some(nonce.as_slice().try_into().map_err(|_| "Invalid pairing commitment.")?);
                Ok(Some(json!({"type":"hello","phase":"reveal","nonce":bytes_to_base64url(&self.nonce),"publicKey":bytes_to_base64url(&self.public)})))
            }
            Some("reveal") => {
                let commitment = self.peer_commitment.ok_or("Pairing reveal preceded commitment.")?;
                let public_text = value.get("publicKey").and_then(Value::as_str).filter(|v|v.len()<=128).ok_or("Invalid pairing public key.")?;
                let public = base64url_to_bytes(public_text)?;
                if nonce.len()!=16 || !same_bytes(&pairing_commitment("mobile", &public, &nonce)?, &commitment) {
                    return Err("Pairing reveal does not match commitment.".into());
                }
                let mut secret = self.identity.derive_secret(&public)?;
                let result: Result<(String, SecureChannel)> = (|| {
                    let verification = verification_code(&self.public, &public, &self.nonce, &nonce, &secret)?;
                    let channel = SecureChannel::new(&secret, &self.nonce, &nonce, true, 2)?;
                    Ok((verification,channel))
                })();
                secret.fill(0);
                let (verification, channel) = result?;
                self.verification = Some(verification); self.peer_public = Some(bytes_to_base64url(&public)); self.channel = Some(channel);
                Ok(None)
            }
            _ => Err("Invalid pairing handshake phase.".into()),
        }
    }
    pub fn verification(&self) -> Option<&str> { self.verification.as_deref() }
    pub fn peer_public_key(&self) -> Option<&str> { self.peer_public.as_deref() }
    pub fn encrypt(&mut self, value: &Value) -> Result<Value> {
        if self.failed { return Err("Pairing session closed.".into()); }
        self.channel.as_mut().ok_or("Secure pairing has not completed.")?.seal(value)
    }
    pub fn decrypt(&mut self, value: &Value) -> Result<Value> {
        if self.failed { return Err("Pairing session closed.".into()); }
        let result = self.channel.as_mut().ok_or("Secure pairing has not completed.")?.open(value);
        if result.is_err() { self.failed = true; self.channel = None; }
        result
    }
}
