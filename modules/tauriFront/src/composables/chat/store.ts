/**
 * chat 领域 store 注册表
 *
 * ChatWindow 被拆为「composables 状态层 + 原子子组件」后,跨组件共享的状态
 * 统一由 ChatWindow 实例创建(每个会话页签一份,KeepAlive 多实例安全),
 * 通过 provide/inject 下发到子组件树。
 *
 * 这里只定义注入 key 与类型,类型由各 composable 的返回类型推导。
 */
import type { InjectionKey } from 'vue'
import type { useChatCore } from './useChatCore'
import type { useChatStreaming } from './useChatStreaming'
import type { useChatCompression } from './useChatCompression'
import type { useMessageMenu } from './useMessageMenu'
import type { useImagePreview } from './useImagePreview'
import type { useAutoScroll } from './useAutoScroll'
import type { useAskUser } from './useAskUser'
import type { useVersioning } from './useVersioning'
import type { useChatSend } from './useChatSend'

export interface ChatStore {
  core: ReturnType<typeof useChatCore>
  streaming: ReturnType<typeof useChatStreaming>
  compression: ReturnType<typeof useChatCompression>
  menu: ReturnType<typeof useMessageMenu>
  preview: ReturnType<typeof useImagePreview>
  autoscroll: ReturnType<typeof useAutoScroll>
  askUser: ReturnType<typeof useAskUser>
  versioning: ReturnType<typeof useVersioning>
  send: ReturnType<typeof useChatSend>
}

export const CHAT_STORE_KEY: InjectionKey<ChatStore> = Symbol('chat-store')
