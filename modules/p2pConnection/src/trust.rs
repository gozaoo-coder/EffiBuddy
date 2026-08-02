//! 信任库：持久化已配对设备的公钥与角色配置。
//!
//! # 角色
//! 首次配对时双方交换 Ed25519 公钥并写入信任库，之后所有连接均用持久化公钥
//! 校验握手签名，杜绝中间人。信任库文件位于 `<appdata>/p2p/trust.json`。
//!
//! # 并发
//! `RwLock` 读多写少：`load`/`list` 读锁，`upsert`/`remove` 写锁。
//! 临界区仅做内存 HashMap 读写 + JSON 序列化，无网络 IO。
//! 文件写入用临时文件 + rename 原子替换，避免半写损坏。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use effisuite_core::{CoreError, Result};

use crate::crypto::{hex_decode, hex_encode, PUBKEY_LEN};

/// 配对角色（决定数据流向）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairRole {
    /// 镜像模式：双向同步聊天/插件/用户缓存
    Mirror,
    /// 主机模式：本机为持久化主机，对端为瘦客户端
    Host,
    /// 副本模式：对端为主机，本机为瘦客户端
    Replica,
}

/// 已配对设备的可信记录
///
/// 字段按大小降序：String(24) → Option<u64>/u64(8) → PairRole(1, Copy)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedPeer {
    /// 设备 id（与 Device.id 一致，由配对时双方协商或采用公钥 hex 前 16 字符）
    pub device_id: String,
    /// 设备展示名（如"电脑"/"手机"）
    pub name: String,
    /// 对端 Ed25519 公钥 hex（32 字节 → 64 hex 字符）
    pub pubkey_hex: String,
    /// 最近已知地址（IP:port 或 host:port）
    pub address: String,
    /// 配对时间（Unix 秒）
    pub paired_at: u64,
    /// 最近在线时间（Unix 秒）
    pub last_seen: u64,
    /// 配对角色
    pub role: PairRole,
}

impl TrustedPeer {
    /// 解析公钥 hex 为 32 字节数组
    pub fn pubkey_bytes(&self) -> Result<[u8; PUBKEY_LEN]> {
        let raw = hex_decode(&self.pubkey_hex)?;
        if raw.len() != PUBKEY_LEN {
            return Err(CoreError::P2p(format!(
                "pubkey length mismatch: expected {PUBKEY_LEN}, got {}",
                raw.len()
            )));
        }
        let mut out = [0u8; PUBKEY_LEN];
        out.copy_from_slice(&raw);
        Ok(out)
    }

    /// 从公钥字节构造 hex
    #[inline]
    pub fn pubkey_to_hex(pubkey: &[u8; PUBKEY_LEN]) -> String {
        hex_encode(pubkey)
    }
}

/// 信任库持久化结构
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct TrustFile {
    /// 本机 device_id（首次启动生成）
    self_device_id: String,
    /// 本机私钥种子 hex（32 字节）
    self_seed_hex: String,
    /// 已配对设备表：device_id → TrustedPeer
    peers: HashMap<String, TrustedPeer>,
}

/// 信任库（线程安全，可廉价 clone）
#[derive(Clone)]
pub struct TrustStore {
    inner: Arc<RwLock<TrustFile>>,
    path: PathBuf,
}

impl TrustStore {
    /// 加载或新建信任库。路径不存在时生成新身份并持久化。
    pub async fn load_or_create(path: PathBuf) -> Result<Self> {
        if path.exists() {
            let data = std::fs::read_to_string(&path).map_err(CoreError::Io)?;
            let file: TrustFile = serde_json::from_str(&data)?;
            Ok(Self {
                inner: Arc::new(RwLock::new(file)),
                path,
            })
        } else {
            // 首次启动：生成身份
            let identity = crate::crypto::IdentityKey::generate();
            let self_device_id = format!("dev-{}", &hex_encode(&identity.public_bytes())[..8]);
            let file = TrustFile {
                self_device_id,
                self_seed_hex: hex_encode(&identity.to_seed()),
                peers: HashMap::new(),
            };
            let store = Self {
                inner: Arc::new(RwLock::new(file)),
                path: path.clone(),
            };
            let file_snapshot = store.inner.read().await.clone();
            store.persist(&file_snapshot).await?;
            Ok(store)
        }
    }

    /// 占位信任库：仅用于 P2pManager 在 `start_with_trust` 之前的字段初始化。
    /// 不持久化到磁盘（路径指向 `/dev/null` 等价空路径），不可用于真实配对。
    /// 调用方应在 `start_with_trust` 时用正式 trust store 替换 manager 字段。
    pub fn placeholder() -> Self {
        let identity = crate::crypto::IdentityKey::generate();
        let self_device_id = format!("dev-anon-{}", &hex_encode(&identity.public_bytes())[..8]);
        let file = TrustFile {
            self_device_id,
            self_seed_hex: hex_encode(&identity.to_seed()),
            peers: HashMap::new(),
        };
        Self {
            inner: Arc::new(RwLock::new(file)),
            path: PathBuf::new(),
        }
    }

    /// 本机 device_id
    pub async fn self_device_id(&self) -> String {
        self.inner.read().await.self_device_id.clone()
    }

    /// 本机身份密钥（每次调用从种子恢复，避免长期持有私钥在内存）
    pub async fn self_identity(&self) -> Result<crate::crypto::IdentityKey> {
        let seed_hex = self.inner.read().await.self_seed_hex.clone();
        let raw = hex_decode(&seed_hex)?;
        if raw.len() != PUBKEY_LEN {
            return Err(CoreError::P2p("self seed length mismatch".to_string()));
        }
        let mut seed = [0u8; PUBKEY_LEN];
        seed.copy_from_slice(&raw);
        Ok(crate::crypto::IdentityKey::from_seed(&seed))
    }

    /// 列出所有已配对设备
    pub async fn list_peers(&self) -> Vec<TrustedPeer> {
        let inner = self.inner.read().await;
        // Vec::values + collect，避免显式索引循环
        inner.peers.values().cloned().collect()
    }

    /// 查询指定设备
    pub async fn get_peer(&self, device_id: &str) -> Option<TrustedPeer> {
        self.inner.read().await.peers.get(device_id).cloned()
    }

    /// 插入或更新可信设备（配对成功 / 信息变更时调用）
    pub async fn upsert_peer(&self, peer: TrustedPeer) -> Result<()> {
        let inner = {
            let mut inner = self.inner.write().await;
            inner.peers.insert(peer.device_id.clone(), peer);
            // 临界区极短：仅做 HashMap insert，无 IO
            // clone 一份用于持久化（持久化在锁外，避免锁内 IO）
            inner.clone()
        };
        self.persist(&inner).await
    }

    /// 更新最近在线时间
    pub async fn touch_last_seen(&self, device_id: &str, ts: u64) -> Result<()> {
        let inner = {
            let mut inner = self.inner.write().await;
            if let Some(p) = inner.peers.get_mut(device_id) {
                p.last_seen = ts;
            }
            inner.clone()
        };
        self.persist(&inner).await
    }

    /// 移除可信设备（取消配对）
    pub async fn remove_peer(&self, device_id: &str) -> Result<()> {
        let inner = {
            let mut inner = self.inner.write().await;
            inner.peers.remove(device_id);
            inner.clone()
        };
        self.persist(&inner).await
    }

    /// 原子持久化（临时文件 + rename）
    async fn persist(&self, file: &TrustFile) -> Result<()> {
        // placeholder 路径为空，跳过持久化
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(file)?;
        // 异步写：用 spawn_blocking 避免阻塞 runtime（文件 IO）
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(CoreError::Io)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, json).map_err(CoreError::Io)?;
            std::fs::rename(&tmp, &path).map_err(CoreError::Io)?;
            Ok(())
        })
        .await
        .map_err(|e| CoreError::P2p(format!("persist task join: {e}")))??;
        Ok(())
    }
}

// ── 单元测试 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::IdentityKey;

    #[tokio::test]
    async fn trust_store_load_or_create_generates_identity() {
        let tmp = tempfile_path();
        let store = TrustStore::load_or_create(tmp.clone()).await.unwrap();
        let dev_id = store.self_device_id().await;
        assert!(dev_id.starts_with("dev-"));
        assert!(tmp.exists());

        // 重新加载：身份稳定
        let store2 = TrustStore::load_or_create(tmp.clone()).await.unwrap();
        assert_eq!(store2.self_device_id().await, dev_id);
        let id1 = store.self_identity().await.unwrap();
        let id2 = store2.self_identity().await.unwrap();
        assert_eq!(id1.public_bytes(), id2.public_bytes());
    }

    #[tokio::test]
    async fn trust_store_upsert_get_remove_peer() {
        let tmp = tempfile_path();
        let store = TrustStore::load_or_create(tmp).await.unwrap();

        let identity = IdentityKey::generate();
        let peer = TrustedPeer {
            device_id: "dev-aaa".to_string(),
            name: "电脑".to_string(),
            pubkey_hex: TrustedPeer::pubkey_to_hex(&identity.public_bytes()),
            address: "192.168.1.10:47823".to_string(),
            paired_at: 1000,
            last_seen: 1000,
            role: PairRole::Mirror,
        };
        store.upsert_peer(peer.clone()).await.unwrap();
        assert_eq!(store.list_peers().await.len(), 1);
        assert_eq!(store.get_peer("dev-aaa").await.unwrap().name, "电脑");

        // touch_last_seen
        store.touch_last_seen("dev-aaa", 2000).await.unwrap();
        assert_eq!(store.get_peer("dev-aaa").await.unwrap().last_seen, 2000);

        // remove
        store.remove_peer("dev-aaa").await.unwrap();
        assert!(store.get_peer("dev-aaa").await.is_none());
        assert!(store.list_peers().await.is_empty());
    }

    #[tokio::test]
    async fn trusted_peer_pubkey_bytes_roundtrip() {
        let identity = IdentityKey::generate();
        let pubkey = identity.public_bytes();
        let hex = TrustedPeer::pubkey_to_hex(&pubkey);
        let peer = TrustedPeer {
            device_id: "x".to_string(),
            name: "x".to_string(),
            pubkey_hex: hex,
            address: "x".to_string(),
            paired_at: 0,
            last_seen: 0,
            role: PairRole::Host,
        };
        assert_eq!(peer.pubkey_bytes().unwrap(), pubkey);
    }

    /// 测试用临时文件路径（不引入 tempfile crate，手工构造唯一路径）
    fn tempfile_path() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "effisuite-trust-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        // 清理旧文件（若存在）
        let _ = std::fs::remove_file(&p);
        p
    }
}
