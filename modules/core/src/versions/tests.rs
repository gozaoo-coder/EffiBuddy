//! 会话版本控制单元测试（git 风格行为验证）
//!
//! 覆盖：追加提交链 / 按消息定位提交 / 回溯 / 撤回 / 开启分支 / 临时版本 /
//! 检出 / 删除引用 / 列表。直接针对 [`super::store::VersionStore`] 与
//! [`super::types`] 的结构测试，不依赖外部 IO。

use super::store::VersionStore;
use crate::versions::CommitKind;
use crate::{Message, Role};

fn tmp_root() -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("effisuite-ver-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 构造连续追加的会话：返回 (store, conv_id, [m1..m4] 消息 id)
async fn seed(store: &VersionStore, conv: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut acc: Vec<Message> = Vec::new();
    for (i, role) in [Role::User, Role::Assistant, Role::User, Role::Assistant]
        .iter()
        .enumerate()
    {
        let id = format!("m{}", i + 1);
        acc.push(Message::new(id.clone(), *role, format!("content {}", i + 1), 1000 + i as u64));
        store.commit_append(conv, &acc, 1000 + i as u64).await.unwrap();
        ids.push(id);
    }
    ids
}

#[tokio::test]
async fn append_creates_chain_and_list() {
    let root = tmp_root();
    let store = VersionStore::new(root.join("v")).unwrap();
    let ids = seed(&store, "c1").await;

    let list = store.list_versions("c1").await.unwrap();
    assert_eq!(list.head, "main");
    // 4 次 append → 4 个提交（新→旧），head 是最新消息 m4
    assert_eq!(list.commits.len(), 4);
    assert_eq!(list.commits[0].head_message_id, ids[3]);
    assert!(list.commits[0].is_head);
    assert_eq!(list.commits[3].head_message_id, ids[0]);
    // refs 只含 main
    assert_eq!(list.refs.len(), 1);
    assert_eq!(list.refs[0].name, "main");
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn rollback_restores_snapshot_and_keeps_checkpoint() {
    let root = tmp_root();
    let store = VersionStore::new(root.join("v")).unwrap();
    let ids = seed(&store, "c1").await;

    // 回溯到 m2：消息应为 [m1, m2]
    let r = store.rollback_to_message("c1", &ids[1], 9000).await.unwrap();
    assert_eq!(r.kind, CommitKind::Rollback);
    assert_eq!(r.messages.len(), 2);
    assert_eq!(r.messages[0].id, ids[0]);
    assert_eq!(r.messages[1].id, ids[1]);

    // 回溯后 HEAD 提交链只剩 2 个
    let list = store.list_versions("c1").await.unwrap();
    assert_eq!(list.commits.len(), 2);
    // 破坏性操作前自动保存了 chkpt-* 检查点
    assert!(list.refs.iter().any(|r| r.kind == "checkpoint"));
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn undo_before_removes_message_and_after() {
    let root = tmp_root();
    let store = VersionStore::new(root.join("v")).unwrap();
    let ids = seed(&store, "c1").await;

    // 撤回至 m3 前：消息应为 [m1, m2]（m3 与 m4 被丢弃）
    let r = store.undo_before_message("c1", &ids[2], 9000).await.unwrap();
    assert_eq!(r.kind, CommitKind::Undo);
    assert_eq!(r.messages.len(), 2);
    assert_eq!(r.messages[1].id, ids[1]);

    // 对首条消息撤回 → 报错（之前没有内容）
    let err = store.undo_before_message("c1", &ids[0], 9001).await;
    assert!(err.is_err());
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn branch_switches_head_and_keeps_original() {
    let root = tmp_root();
    let store = VersionStore::new(root.join("v")).unwrap();
    let ids = seed(&store, "c1").await;

    // 从 m2 开分支：工作区变为 [m1, m2]，HEAD 切到新分支
    let r = store.create_branch("c1", &ids[1], 9000).await.unwrap();
    assert_eq!(r.kind, CommitKind::Branch);
    assert!(r.branch.starts_with("branch-"));
    assert_eq!(r.messages.len(), 2);

    // 新分支上继续追加 m5
    let mut acc = r.messages.clone();
    acc.push(Message::new("m5", Role::User, "content 5", 9001));
    store.commit_append("c1", &acc, 9001).await.unwrap();

    // 列表：head 分支为新分支；refs 同时含 main（m4）与新分支（m5）
    let list = store.list_versions("c1").await.unwrap();
    assert!(list.head.starts_with("branch-"));
    assert_eq!(list.commits.len(), 3); // m5 → m2 → m1
    assert!(list.refs.iter().any(|r| r.name == "main"));
    assert!(list.refs.iter().any(|r| r.name == list.head));

    // 检出回 main：工作区恢复 [m1..m4]
    let r = store.checkout_ref("c1", "main", 9002).await.unwrap();
    assert_eq!(r.branch, "main");
    assert_eq!(r.messages.len(), 4);
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn temp_version_bookmark_and_delete_ref() {
    let root = tmp_root();
    let store = VersionStore::new(root.join("v")).unwrap();
    let ids = seed(&store, "c1").await;

    // 在 m2 处保存临时版本（带备注）
    let t = store
        .save_temp_version("c1", &ids[1], "重要节点".to_string(), 9000)
        .await
        .unwrap();
    assert!(t.name.starts_with("temp-"));
    assert_eq!(t.note, "重要节点");

    // 不移动 HEAD（仍是 m4）
    let list = store.list_versions("c1").await.unwrap();
    assert_eq!(list.commits[0].head_message_id, ids[3]);

    // 检出临时版本：从该提交新建分支继续
    let r = store.checkout_ref("c1", &t.name, 9001).await.unwrap();
    assert!(r.branch.starts_with("branch-"));
    assert_eq!(r.messages.len(), 2);

    // 删除临时版本引用（此时 HEAD 已切走，允许删除）
    store.delete_ref("c1", &t.name).await.unwrap();
    let list = store.list_versions("c1").await.unwrap();
    assert!(!list.refs.iter().any(|r| r.name == t.name));

    // main 不可删除
    let err = store.delete_ref("c1", "main").await;
    assert!(err.is_err());
    std::fs::remove_dir_all(&root).ok();
}

#[tokio::test]
async fn conversation_store_with_versions_end_to_end() {
    use crate::storage::ConversationStore;
    let root = tmp_root();
    let store = ConversationStore::with_versions(root.join("conv")).unwrap();

    store
        .append_message("c1", Message::new("a", Role::User, "A", 1), 1)
        .await
        .unwrap();
    store
        .append_message("c1", Message::new("b", Role::Assistant, "B", 2), 2)
        .await
        .unwrap();
    store
        .append_message("c1", Message::new("c", Role::User, "C", 3), 3)
        .await
        .unwrap();

    // 列表有 3 个提交
    let list = store.version_list("c1").await.unwrap();
    assert_eq!(list.commits.len(), 3);

    // 回溯到 b：工作区与仓库同步为 [A, B]
    let r = store.version_rollback("c1", "b", 9000).await.unwrap();
    assert_eq!(r.messages.len(), 2);
    let conv = store.load("c1").await.unwrap().unwrap();
    assert_eq!(conv.messages.len(), 2);
    assert_eq!(conv.messages[1].id, "b");

    // 删除会话联动清理版本仓库
    store.delete("c1").await.unwrap();
    let list = store.version_list("c1").await.unwrap();
    assert!(list.commits.is_empty());
    std::fs::remove_dir_all(&root).ok();
}
