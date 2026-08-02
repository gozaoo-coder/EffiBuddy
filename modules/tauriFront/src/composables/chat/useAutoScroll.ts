/**
 * 消息列表自动滚动控制
 *
 * markstream-vue 的流式渲染是异步的,nextTick 后 DOM 可能尚未增长,
 * scrollHeight 是旧值导致 scrollBottom 失效。改用 MutationObserver 监听
 * scroller 子树变化 + requestAnimationFrame 节流跟随底部。
 * stickToBottom 跟踪用户滚动位置:上滑阅读时暂停跟随,滑回底部恢复。
 *
 * 进入会话场景的特殊处理:
 * - jumpToBottom:强制滚动到底部,无视当前 sticky 状态,用于初次进入会话。
 * - pinToBottom(durationMs):开启一个时间窗口,期间所有 DOM 变化都强制
 *   跟随底部,覆盖 markstream-vue 异步渲染 / 图片解码 / 附件 base64 回填
 *   等延迟增长,确保进入会话后稳定停在最新消息处。
 */
import { ref } from 'vue'

export function useAutoScroll() {
  const scroller = ref<HTMLElement | null>(null)
  const stickToBottom = ref(true)
  let mutationObserver: MutationObserver | null = null
  let resizeObserver: ResizeObserver | null = null
  let scrollRafId: number | null = null
  /** pin 窗口结束时间戳(performance.now() 基准;0 表示未开启 pin) */
  let pinUntil = 0
  /** pin 窗口兜底 timer:窗口结束后再滚动一次,确保最终位置正确 */
  let pinTimer: number | null = null
  /** 图片 load 监听是否已绑定(capture 模式,需在解绑时同步移除) */
  let imageLoadBound = false

  function applyScrollBottom() {
    const el = scroller.value
    if (el) el.scrollTop = el.scrollHeight
  }

  function scrollBottom() {
    if (stickToBottom.value) applyScrollBottom()
  }

  /** 强制滚动到底部:无视当前 sticky 状态,并把 sticky 重置为 true
   *  用于初次进入会话 / 切换会话等必须展示最新消息的场景 */
  function jumpToBottom() {
    stickToBottom.value = true
    applyScrollBottom()
  }

  /** 节流跟随底部:observer 触发时合并到下一帧统一滚动,避免高频 token 抖动 */
  function scheduleFollowBottom() {
    const inPinWindow = performance.now() < pinUntil
    // pin 窗口内强制跟随;否则仅在 sticky 状态下跟随
    if (!inPinWindow && !stickToBottom.value) return
    if (scrollRafId !== null) return
    scrollRafId = requestAnimationFrame(() => {
      scrollRafId = null
      // 二次校验:RAF 触发时 pin 窗口可能已结束
      if (performance.now() < pinUntil || stickToBottom.value) applyScrollBottom()
    })
  }

  /** 在指定窗口期内强制跟随底部(覆盖异步渲染延迟)
   *  典型场景:进入会话加载历史消息后,markstream-vue / 附件 / 图片
   *  在数百 ms 内持续增长高度,需要持续跟随到底部 */
  function pinToBottom(durationMs = 800) {
    pinUntil = performance.now() + durationMs
    // 立即滚动一次
    applyScrollBottom()
    // 兜底:窗口结束后再滚动一次,确保最终位置正确
    if (pinTimer !== null) clearTimeout(pinTimer)
    pinTimer = window.setTimeout(() => {
      pinTimer = null
      pinUntil = 0
      applyScrollBottom()
    }, durationMs)
  }

  /** 滚动事件:用户上滑超过阈值时停止跟随,滑回底部时恢复
   *  注意:pin 窗口期内不更新 sticky,避免异步渲染期间的微小滚动误判 */
  function onScrollerScroll() {
    if (performance.now() < pinUntil) return
    const el = scroller.value
    if (!el) return
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight
    stickToBottom.value = distance < 80
  }

  /** 图片 load 事件(capture 冒泡):历史消息图片解码完成后高度增长,需重新跟随 */
  function onImageLoad() {
    // pin 窗口内或 sticky 状态下才跟随,避免用户上滑阅读时被打断
    if (performance.now() < pinUntil || stickToBottom.value) {
      applyScrollBottom()
    }
  }

  /** ResizeObserver 回调:覆盖 markstream-vue 内部异步高度增长 / 折叠展开等场景 */
  function onContentSizeChange() {
    if (performance.now() < pinUntil || stickToBottom.value) {
      applyScrollBottom()
    }
  }

  /** 在 scroller 挂载/卸载时绑定/解绑 observer 与滚动监听 */
  function attachScroller(el: HTMLElement | null, oldEl?: HTMLElement | null) {
    if (oldEl) {
      oldEl.removeEventListener('scroll', onScrollerScroll)
      if (imageLoadBound) {
        oldEl.removeEventListener('load', onImageLoad, true)
        imageLoadBound = false
      }
    }
    if (mutationObserver) {
      mutationObserver.disconnect()
      mutationObserver = null
    }
    if (resizeObserver) {
      resizeObserver.disconnect()
      resizeObserver = null
    }
    if (!el) return
    el.addEventListener('scroll', onScrollerScroll, { passive: true })
    // capture 模式监听后代 img 的 load 事件(冒泡阶段捕获)
    el.addEventListener('load', onImageLoad, true)
    imageLoadBound = true
    mutationObserver = new MutationObserver(scheduleFollowBottom)
    mutationObserver.observe(el, {
      childList: true,
      subtree: true,
      characterData: true,
    })
    // ResizeObserver 兜底:监听 scroller 直接子节点(消息气泡)的尺寸变化,
    // 覆盖 markstream-vue 异步渲染导致的 scrollHeight 增长
    resizeObserver = new ResizeObserver(onContentSizeChange)
    for (const child of el.children) {
      resizeObserver.observe(child)
    }
    // 新挂载或重新挂载时:若在 pin 窗口或 sticky 状态,立即滚动一次
    // (覆盖 ChatHome → ChatMessageList 切换时 scroller 才挂载的场景)
    if (performance.now() < pinUntil || stickToBottom.value) {
      applyScrollBottom()
    }
  }

  /** 组件卸载时清理(幂等) */
  function dispose() {
    attachScroller(null)
    if (scrollRafId !== null) {
      cancelAnimationFrame(scrollRafId)
      scrollRafId = null
    }
    if (pinTimer !== null) {
      clearTimeout(pinTimer)
      pinTimer = null
    }
    pinUntil = 0
  }

  return { scroller, stickToBottom, scrollBottom, jumpToBottom, pinToBottom, attachScroller, dispose }
}
