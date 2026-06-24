<template>
  <div class="analog-clock">
    <svg :viewBox="`0 0 ${size} ${size}`" :width="size" :height="size">
      <circle :cx="cx" :cy="cy" :r="radius" class="face" />
      <!-- hour ticks -->
      <line
        v-for="i in 12"
        :key="`h-${i}`"
        :x1="tickX(i - 1, radius - 4)"
        :y1="tickY(i - 1, radius - 4)"
        :x2="tickX(i - 1, radius - 10)"
        :y2="tickY(i - 1, radius - 10)"
        class="tick-hour"
      />
      <!-- minute ticks -->
      <line
        v-for="i in 60"
        v-show="i % 5 !== 0"
        :key="`m-${i}`"
        :x1="tickX(i - 1, radius - 4)"
        :y1="tickY(i - 1, radius - 4)"
        :x2="tickX(i - 1, radius - 7)"
        :y2="tickY(i - 1, radius - 7)"
        class="tick-min"
      />
      <!-- hands -->
      <line :x1="cx" :y1="cy" :x2="hourX" :y2="hourY" class="hand hour" />
      <line :x1="cx" :y1="cy" :x2="minX" :y2="minY" class="hand min" />
      <line :x1="cx" :y1="cy" :x2="secX" :y2="secY" class="hand sec" />
      <circle :cx="cx" :cy="cy" r="3" class="pivot" />
    </svg>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'

const size = 180
const cx = size / 2
const cy = size / 2
const radius = size / 2 - 6

const now = ref(new Date())
let timer: ReturnType<typeof setInterval> | null = null

const hours = computed(() => now.value.getHours() % 12)
const minutes = computed(() => now.value.getMinutes())
const seconds = computed(() => now.value.getSeconds())

function angle(unit: number, total: number) {
  return (unit / total) * Math.PI * 2 - Math.PI / 2
}
function handX(unit: number, total: number, len: number) {
  return cx + Math.cos(angle(unit, total)) * len
}
function handY(unit: number, total: number, len: number) {
  return cy + Math.sin(angle(unit, total)) * len
}

const hourX = computed(() => handX(hours.value * 5 + minutes.value / 12, 60, radius * 0.5))
const hourY = computed(() => handY(hours.value * 5 + minutes.value / 12, 60, radius * 0.5))
const minX = computed(() => handX(minutes.value + seconds.value / 60, 60, radius * 0.75))
const minY = computed(() => handY(minutes.value + seconds.value / 60, 60, radius * 0.75))
const secX = computed(() => handX(seconds.value, 60, radius * 0.85))
const secY = computed(() => handY(seconds.value, 60, radius * 0.85))

function tickX(i: number, r: number) {
  return cx + Math.cos((i / 12) * Math.PI * 2 - Math.PI / 2) * r
}
function tickY(i: number, r: number) {
  return cy + Math.sin((i / 12) * Math.PI * 2 - Math.PI / 2) * r
}

onMounted(() => {
  timer = setInterval(() => (now.value = new Date()), 1000)
})
onUnmounted(() => {
  if (timer) clearInterval(timer)
})
</script>

<style scoped>
.analog-clock {
  display: flex;
  align-items: center;
  justify-content: center;
}
.face {
  fill: rgba(30, 30, 46, 0.6);
  stroke: rgba(255, 255, 255, 0.15);
  stroke-width: 2;
}
.tick-hour {
  stroke: #cdd6f4;
  stroke-width: 2;
}
.tick-min {
  stroke: #6c7086;
  stroke-width: 1;
}
.hand {
  stroke-linecap: round;
}
.hand.hour {
  stroke: #cdd6f4;
  stroke-width: 4;
}
.hand.min {
  stroke: #cdd6f4;
  stroke-width: 2.5;
}
.hand.sec {
  stroke: #f38ba8;
  stroke-width: 1.5;
}
.pivot {
  fill: #cdd6f4;
}
</style>
