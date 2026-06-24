<template>
  <div class="digital-clock">
    <div class="time">{{ time }}</div>
    <div class="meta">
      <span>{{ date }}</span>
      <span class="dot">·</span>
      <span>{{ weekday }}</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'

const time = ref('--:--:--')
const date = ref('----/--/--')
const weekday = ref('---')
let timer: ReturnType<typeof setInterval> | null = null

function tick() {
  const now = new Date()
  const pad = (n: number) => n.toString().padStart(2, '0')
  time.value = `${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`
  date.value = `${now.getFullYear()}/${pad(now.getMonth() + 1)}/${pad(now.getDate())}`
  weekday.value = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'][now.getDay()]
}

onMounted(() => {
  tick()
  timer = setInterval(tick, 1000)
})
onUnmounted(() => {
  if (timer) clearInterval(timer)
})
</script>

<style scoped>
.digital-clock {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 8px 12px;
  color: #cdd6f4;
  font-variant-numeric: tabular-nums;
}
.time {
  font-size: 28px;
  font-weight: 600;
  letter-spacing: 1px;
}
.meta {
  font-size: 11px;
  color: #a6adc8;
  margin-top: 2px;
  display: flex;
  gap: 6px;
}
.dot {
  color: #6c7086;
}
</style>
