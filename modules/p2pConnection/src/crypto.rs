//! P2P 加密原语：身份密钥、配对签名、动态会话密钥协商与对称加解密。
//!
//! # 密码学设计
//! - **身份密钥**：每个设备持有一对 Ed25519 长期密钥（[`IdentityKey`]），
//!   公钥即设备指纹，私钥永不离开本设备。配对时双方交换并持久化对方公钥（信任库）。
//! - **配对签名**：首次配对交换可信密钥后，所有后续连接的 `Hello`/`HelloAck`
//!   均由 Ed25519 私钥签名，对端用持久化的公钥校验，防止中间人冒充。
//! - **动态会话密钥**：每次 TCP 连接双方各生成一次性 X25519 临时密钥对，
//!   经 ECDH 协商出共享秘密，再经 HKDF-SHA256 派生为 AES-256-GCM 密钥。
//!   会话密钥仅存活于单次连接，**前向保密**：即便长期私钥日后泄露，旧会话流量也无法解密。
//! - **对称加密**：AES-256-GCM，12 字节随机 nonce，帧内附带 nonce + 密文 + 16 字节 tag。
//!
//! # 内存与并发
//! - 密钥结构体为定长数组（栈分配，零堆分配），`Clone` 走 memcpy。
//! - 所有方法无锁、无 IO，可被任意线程并发调用（`Send + Sync`）。
//! - 不在热路径使用 `clone()`（密钥仅在手握与持久化时 clone）。

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey, SigningKey};
use hkdf::Hkdf;
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey, StaticSecret};

use effisuite_core::{CoreError, Result};

/// Ed25519 公钥字节数（32 字节）
pub const PUBKEY_LEN: usize = 32;
/// Ed25519 签名字节数（64 字节）
pub const SIGNATURE_LEN: usize = 64;
/// X25519 公钥字节数（32 字节）
pub const EPHEMERAL_PUB_LEN: usize = 32;
/// AES-256-GCM 密钥字节数（32 字节）
pub const SESSION_KEY_LEN: usize = 32;
/// AES-256-GCM nonce 字节数（12 字节）
pub const NONCE_LEN: usize = 12;
/// HKDF info 标签（防止跨用途密钥复用）
const HKDF_INFO: &[u8] = b"effisuite-p2p-session-v1";

/// 设备身份密钥对（Ed25519）
///
/// 长期密钥，公钥即设备指纹。私钥仅在内存与本地持久化文件中存在，永不通过网络传输。
/// 字段顺序：signing（含 32B 私钥 + 派生的 32B 公钥，内部为定长数组）→ verifying（32B）。
#[derive(Clone)]
pub struct IdentityKey {
    /// Ed25519 签名密钥（含私钥与公钥）
    signing: SigningKey,
}

impl IdentityKey {
    /// 生成新的身份密钥对（使用 OsRng，密码学安全）
    #[inline]
    pub fn generate() -> Self {
        let mut rng = OsRng;
        Self {
            signing: SigningKey::generate(&mut rng),
        }
    }

    /// 从 32 字节私钥种子恢复身份密钥
    pub fn from_seed(seed: &[u8; PUBKEY_LEN]) -> Self {
        Self {
            signing: SigningKey::from_bytes(seed),
        }
    }

    /// 导出 32 字节私钥种子（用于持久化）
    #[inline]
    pub fn to_seed(&self) -> [u8; PUBKEY_LEN] {
        self.signing.to_bytes()
    }

    /// 导出 32 字节公钥（设备指纹）
    #[inline]
    pub fn public_bytes(&self) -> [u8; PUBKEY_LEN] {
        self.signing.verifying_key().to_bytes()
    }

    /// 用私钥对消息签名，返回 64 字节签名
    #[inline]
    pub fn sign(&self, msg: &[u8]) -> [u8; SIGNATURE_LEN] {
        self.signing.sign(msg).to_bytes()
    }

    /// 构造待签名握手消息：`[ephemeral_pub || ts_be(8) || device_id]`
    /// 固定布局便于对端无需 JSON 即可重组验证。
    pub fn handshake_signed_payload(
        ephemeral_pub: &[u8; EPHEMERAL_PUB_LEN],
        timestamp: u64,
        device_id: &str,
    ) -> Vec<u8> {
        let mut buf = Vec::with_capacity(EPHEMERAL_PUB_LEN + 8 + device_id.len());
        buf.extend_from_slice(ephemeral_pub);
        buf.extend_from_slice(&timestamp.to_be_bytes());
        buf.extend_from_slice(device_id.as_bytes());
        buf
    }
}

impl std::fmt::Debug for IdentityKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdentityKey")
            .field("public", &hex_short(&self.public_bytes()))
            .finish()
    }
}

/// 验证 Ed25519 签名
///
/// `peer_pubkey` 为对端持久化的 32 字节公钥（信任库），`signature` 为 64 字节签名。
pub fn verify_signature(
    peer_pubkey: &[u8; PUBKEY_LEN],
    msg: &[u8],
    signature: &[u8],
) -> std::result::Result<(), ed25519_dalek::SignatureError> {
    let vk = VerifyingKey::from_bytes(peer_pubkey)?;
    let sig = Signature::from_slice(signature)?;
    vk.verify(msg, &sig)
}

/// 一次性 X25519 临时密钥对（用于单次连接的 ECDH）
///
/// 持有 `StaticSecret`（非 `EphemeralSecret`）以便能 `borrow` 出共享秘密而不被消费，
/// 从而支持"既做客户端又做服务端"的对称握手。连接结束后应丢弃。
pub struct EphemeralKeypair {
    secret: StaticSecret,
    public: X25519PublicKey,
}

impl EphemeralKeypair {
    /// 生成新的临时密钥对
    #[inline]
    pub fn generate() -> Self {
        let mut rng = OsRng;
        let secret = StaticSecret::random_from_rng(&mut rng);
        let public = X25519PublicKey::from(&secret);
        Self { secret, public }
    }

    /// 导出 32 字节临时公钥（放入 Hello/HelloAck）
    #[inline]
    pub fn public_bytes(&self) -> [u8; EPHEMERAL_PUB_LEN] {
        self.public.to_bytes()
    }

    /// 与对端临时公钥协商共享秘密，HKDF 派生为 32 字节 AES-256 会话密钥
    ///
    /// 双方调用顺序无关：`Hkdf(local_priv, remote_pub) == Hkdf(remote_priv, local_pub)`。
    /// ikm 用双方公钥按字典序拼接（避免 alice 拼 alice||bob、bob 拼 bob||alice 导致 ikm 不同）。
    pub fn derive_session_key(&self, peer_pub: &[u8; EPHEMERAL_PUB_LEN]) -> [u8; SESSION_KEY_LEN] {
        let peer = X25519PublicKey::from(*peer_pub);
        let shared = self.secret.diffie_hellman(&peer);
        // HKDF-SHA256：salt = 共享秘密（32B），ikm = 双方公钥按字典序拼接（防未知密钥共享攻击 + 双方一致）
        let hk = Hkdf::<Sha256>::new(
            Some(shared.as_bytes()),
            &sorted_concat_pubkeys(&self.public_bytes(), peer_pub),
        );
        let mut okm = [0u8; SESSION_KEY_LEN];
        // expand 单次 32B，prk 长度足够，不会失败
        hk.expand(HKDF_INFO, &mut okm)
            .expect("HKDF expand 32B must succeed for Hkdf<Sha256>");
        okm
    }
}

/// 按字典序拼接双方临时公钥作为 HKDF ikm（双方一致，防未知密钥共享攻击）
#[inline]
fn sorted_concat_pubkeys(a: &[u8; EPHEMERAL_PUB_LEN], b: &[u8; EPHEMERAL_PUB_LEN]) -> [u8; 64] {
    let mut out = [0u8; 64];
    if a <= b {
        out[..EPHEMERAL_PUB_LEN].copy_from_slice(a);
        out[EPHEMERAL_PUB_LEN..].copy_from_slice(b);
    } else {
        out[..EPHEMERAL_PUB_LEN].copy_from_slice(b);
        out[EPHEMERAL_PUB_LEN..].copy_from_slice(a);
    }
    out
}

/// AES-256-GCM 会话加密器
///
/// 每次加密生成随机 12B nonce，nonce 与密文一同返回（`nonce || ciphertext || tag`）。
/// 解密时拆出前 12B nonce 再解密。无锁、可并发（内部无状态）。
pub struct SessionCipher {
    cipher: Aes256Gcm,
}

impl SessionCipher {
    /// 从 32 字节会话密钥构造
    pub fn new(session_key: &[u8; SESSION_KEY_LEN]) -> Self {
        let key = Key::<Aes256Gcm>::from_slice(session_key);
        Self {
            cipher: Aes256Gcm::new(key),
        }
    }

    /// 加密：返回 `nonce(12) || ciphertext+tag`
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut rng = OsRng;
        let nonce_bytes = {
            let mut n = [0u8; NONCE_LEN];
            rng.fill_bytes(&mut n);
            n
        };
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(nonce, Payload { msg: plaintext, aad: &[] })
            .map_err(|e| CoreError::P2p(format!("aes-gcm encrypt: {e}")))?;
        // 拼接 nonce || ciphertext（ciphertext 末尾含 16B tag）
        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// 解密：输入为 `nonce(12) || ciphertext+tag`
    pub fn decrypt(&self, frame: &[u8]) -> Result<Vec<u8>> {
        if frame.len() < NONCE_LEN {
            return Err(CoreError::P2p("frame too short for nonce".to_string()));
        }
        let (nonce_bytes, ciphertext) = frame.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);
        self.cipher
            .decrypt(nonce, Payload { msg: ciphertext, aad: &[] })
            .map_err(|e| CoreError::P2p(format!("aes-gcm decrypt: {e}")))
    }
}

/// 生成 8 字节大端时间戳防重放窗口种子（握手消息携带）
#[inline]
pub fn now_ts() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 校验握手时间戳防重放：允许 ±60s 时钟偏移
#[inline]
pub fn ts_within_window(ts: u64, now: u64) -> bool {
    ts.abs_diff(now) <= 60
}

/// 公钥字节转简短 hex（调试用，仅取前 8 字符）
#[inline]
fn hex_short(b: &[u8]) -> String {
    let s = hex_encode(b);
    s.chars().take(8).collect()
}

/// 轻量 hex 编码（避免引入 hex crate）
pub fn hex_encode(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for byte in b {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

/// hex 解码（信任库序列化用）
pub fn hex_decode(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        return Err(CoreError::P2p("hex string odd length".to_string()));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks_exact(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

#[inline]
fn hex_val(c: u8) -> Result<u8> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(CoreError::P2p(format!("invalid hex char: {c}"))),
    }
}

// 防止未使用警告（OsRng/RngCore 在某些路径下可能未被直接引用）
#[allow(dead_code)]
fn _ensure_rng_used() {
    let _ = OsRng.next_u32();
    // EphemeralSecret 仅用于类型文档引用，实际使用 StaticSecret
    let _ = std::marker::PhantomData::<EphemeralSecret>;
}

// ── 单元测试 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_key_sign_verify_roundtrip() {
        let alice = IdentityKey::generate();
        let bob = IdentityKey::generate();
        let msg = b"hello world";
        let sig = alice.sign(msg);
        assert_eq!(sig.len(), SIGNATURE_LEN);

        // 正确公钥验证通过
        assert!(verify_signature(&alice.public_bytes(), msg, &sig).is_ok());
        // 错误公钥验证失败
        assert!(verify_signature(&bob.public_bytes(), msg, &sig).is_err());
        // 篡改消息验证失败
        assert!(verify_signature(&alice.public_bytes(), b"tampered", &sig).is_err());
    }

    #[test]
    fn identity_key_seed_roundtrip() {
        let key = IdentityKey::generate();
        let seed = key.to_seed();
        let restored = IdentityKey::from_seed(&seed);
        assert_eq!(restored.public_bytes(), key.public_bytes());
        // 同一私钥签名应一致（确定性签名：Ed25519 是确定性的）
        let msg = b"test";
        assert_eq!(key.sign(msg), restored.sign(msg));
    }

    #[test]
    fn ecdh_derives_symmetric_session_key() {
        // 双方各自生成临时密钥对，协商出的会话密钥应相等
        let alice = EphemeralKeypair::generate();
        let bob = EphemeralKeypair::generate();
        let alice_key = alice.derive_session_key(&bob.public_bytes());
        let bob_key = bob.derive_session_key(&alice.public_bytes());
        assert_eq!(alice_key, bob_key);
        assert_ne!(alice_key, [0u8; SESSION_KEY_LEN]);
    }

    #[test]
    fn session_cipher_encrypt_decrypt_roundtrip() {
        let alice = EphemeralKeypair::generate();
        let bob = EphemeralKeypair::generate();
        let key = alice.derive_session_key(&bob.public_bytes());
        let cipher = SessionCipher::new(&key);

        let plaintext = br#"{"type":"ping","ts":12345}"#;
        let frame = cipher.encrypt(plaintext).unwrap();
        // nonce(12) + ciphertext + tag(16)
        assert!(frame.len() > NONCE_LEN + 16);

        let decrypted = cipher.decrypt(&frame).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn session_cipher_rejects_tampered_frame() {
        let alice = EphemeralKeypair::generate();
        let bob = EphemeralKeypair::generate();
        let key = alice.derive_session_key(&bob.public_bytes());
        let cipher = SessionCipher::new(&key);

        let mut frame = cipher.encrypt(b"secret").unwrap();
        // 翻转一个密文字节，应解密失败（GCM 完整性校验）
        let last = frame.len() - 1;
        frame[last] ^= 0xff;
        assert!(cipher.decrypt(&frame).is_err());
    }

    #[test]
    fn session_cipher_rejects_short_frame() {
        let key = [1u8; SESSION_KEY_LEN];
        let cipher = SessionCipher::new(&key);
        // < 12 字节
        assert!(cipher.decrypt(&[0u8; 5]).is_err());
    }

    #[test]
    fn different_ephemeral_keys_yield_different_session_keys() {
        let alice1 = EphemeralKeypair::generate();
        let alice2 = EphemeralKeypair::generate();
        let bob = EphemeralKeypair::generate();
        let k1 = alice1.derive_session_key(&bob.public_bytes());
        let k2 = alice2.derive_session_key(&bob.public_bytes());
        assert_ne!(k1, k2);
    }

    #[test]
    fn handshake_signed_payload_deterministic() {
        let eph = [7u8; EPHEMERAL_PUB_LEN];
        let p = IdentityKey::handshake_signed_payload(&eph, 1000, "dev-1");
        assert_eq!(p.len(), EPHEMERAL_PUB_LEN + 8 + 5);
        assert_eq!(&p[..EPHEMERAL_PUB_LEN], &eph);
        assert_eq!(&p[EPHEMERAL_PUB_LEN..EPHEMERAL_PUB_LEN + 8], 1000u64.to_be_bytes());
        assert_eq!(&p[EPHEMERAL_PUB_LEN + 8..], b"dev-1");
    }

    #[test]
    fn ts_window_allows_small_skew_rejects_large() {
        let now = 1000u64;
        assert!(ts_within_window(1000, now));
        assert!(ts_within_window(1059, now));
        assert!(ts_within_window(941, now));
        assert!(!ts_within_window(1061, now));
        assert!(!ts_within_window(939, now));
    }

    #[test]
    fn hex_encode_decode_roundtrip() {
        let original = vec![0x00, 0xff, 0xab, 0x12, 0x9f];
        let s = hex_encode(&original);
        let back = hex_decode(&s).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn hex_decode_rejects_invalid() {
        assert!(hex_decode("abc").is_err()); // 奇数长度
        assert!(hex_decode("xy").is_err()); // 非法字符
    }
}
