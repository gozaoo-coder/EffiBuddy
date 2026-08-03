//! 真实 token 计数（tiktoken cl100k_base BPE）
//!
//! 用于压缩节省量、上下文占用等需要真实 token 数而非"字符数/4"估算的场景。
//! cl100k_base 是 OpenAI 主流模型的 BPE 词表，对中英文都有较好的近似精度；
//! 数据随 crate 打包，离线可用，无需联网。

use std::sync::OnceLock;

use tiktoken_rs::CoreBPE;

use crate::models::Message;

/// 全局共享的 cl100k_base BPE 编码器（懒加载，进程内只初始化一次）
fn bpe() -> &'static CoreBPE {
    static BPE: OnceLock<CoreBPE> = OnceLock::new();
    BPE.get_or_init(|| tiktoken_rs::cl100k_base().expect("加载 tiktoken cl100k_base 失败"))
}

/// 对一段文本做真实 token 计数（cl100k_base，含特殊 token 编码）。
pub fn count_tokens(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    bpe().encode_with_special_tokens(text).len() as u64
}

/// 统计一组消息的历史段 token 占用（与压缩/上下文注入口径一致）：
///
/// - 每条消息的 `content`
/// - 助手消息的 `reasoning`（思维链随历史注入，必须计入）
///
/// 用于"完全未压缩 vs 压缩后"的节省量对比：base 与 current 都按同一口径统计，
/// 得到的差值即压缩释放的真实 token 数，百分比相对原始上下文。
pub fn count_messages_tokens(messages: &[Message]) -> u64 {
    messages
        .iter()
        .map(|m| {
            let mut n = count_tokens(&m.content);
            if let Some(r) = m.reasoning.as_deref() {
                n += count_tokens(r);
            }
            n
        })
        .sum()
}
