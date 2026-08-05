/**
 * pendingPrompt —— 空态聊天框「待发送提示词」单条传递
 *
 * 场景：TabEmpty 空态下还没有任何 ChatWindow（无页签），其聊天框发送时
 * 无法直接调用会话级 store 的 send。方案：
 *   1. TabEmpty 先把提示词写入本模块级 ref；
 *   2. 随即 openTab 新建聊天页签（__new_chat__ 哨兵）；
 *   3. 新 ChatWindow 挂载后读取并消费该提示词，经 sendPrompt 自动发送。
 *
 * 只缓存一条且消费即清空：setPendingPrompt 与 openTab 在同一同步任务内，
 * 新实例挂载必然紧接着发生，不存在跨会话误发的窗口期。
 */
import { ref } from 'vue'

const pendingPrompt = ref('')

/** 写入待发送提示词（TabEmpty 发送前调用） */
export function setPendingPrompt(text: string): void {
  pendingPrompt.value = text
}

/** 读取并清空（ChatWindow 挂载时调用；返回空串表示无待发内容） */
export function consumePendingPrompt(): string {
  const text = pendingPrompt.value
  pendingPrompt.value = ''
  return text
}
