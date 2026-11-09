<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

// The clock here is scheduled the way a real uniwidged clock is: it asks to be
// woken at the top of the next minute, and is not touched in between.
const now = ref<Date | null>(null)
const wakes = ref(0)
let timer: ReturnType<typeof setTimeout> | undefined

function wake() {
  now.value = new Date()
  wakes.value++
  const ms = 60_000 - (Date.now() % 60_000)
  timer = setTimeout(wake, ms + 5)
}

onMounted(wake)
onBeforeUnmount(() => timer && clearTimeout(timer))

const time = computed(() =>
  now.value
    ? now.value.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
    : '--:--',
)
const date = computed(() =>
  now.value
    ? now.value.toLocaleDateString(undefined, { weekday: 'long', day: 'numeric', month: 'long' })
    : ' ',
)

// A minute of CPU history: sixty points, as `widgets/system-load` keeps.
const points = [
  0.22, 0.19, 0.24, 0.31, 0.27, 0.22, 0.2, 0.26, 0.38, 0.52, 0.47, 0.35, 0.29, 0.31, 0.27, 0.24,
  0.21, 0.23, 0.33, 0.41, 0.36, 0.29, 0.25, 0.22, 0.26, 0.3, 0.28, 0.24, 0.23, 0.27,
]
const W = 200
const H = 46
const line = points
  .map((v, i) => `${i === 0 ? 'M' : 'L'}${((i / (points.length - 1)) * W).toFixed(1)} ${(H - v * H * 1.4).toFixed(1)}`)
  .join(' ')
const area = `${line} L${W} ${H} L0 ${H} Z`
const last = H - points[points.length - 1] * H * 1.4
</script>

<template>
  <figure class="uw-desk" aria-label="A preview of uniwidged widgets on a desktop">
    <div class="wall" aria-hidden="true">
      <span class="blob b1" /><span class="blob b2" /><span class="blob b3" />
    </div>

    <!-- bottom layer: the widgets -->
    <div class="w clock">
      <div class="k">{{ date }}</div>
      <div class="big">{{ time }}</div>
    </div>

    <div class="w load">
      <div class="row"><span class="k">CPU</span><span class="chip">metrics</span></div>
      <div class="mid">27%</div>
      <div class="bar"><i style="width: 27%" /></div>
      <svg :viewBox="`0 0 ${W} ${H}`" class="spark" aria-hidden="true">
        <defs>
          <linearGradient id="uw-spark" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stop-color="#4f7bff" stop-opacity=".45" />
            <stop offset="1" stop-color="#4f7bff" stop-opacity="0" />
          </linearGradient>
        </defs>
        <path :d="area" fill="url(#uw-spark)" />
        <path :d="line" fill="none" stroke="#86a3ff" stroke-width="2" stroke-linejoin="round" />
        <circle :cx="W" :cy="last" r="4" fill="#ffb547" />
      </svg>
    </div>

    <div class="w batt">
      <div class="k">Battery</div>
      <div class="mid">87%</div>
      <div class="sub">2 h 40 min left</div>
    </div>

    <!-- an ordinary window, above them -->
    <div class="win" aria-hidden="true">
      <div class="bar-top"><i /><i /><i /></div>
      <pre><span class="p">$</span> uniwidged status
  desktop 4127 since 09:12
  shell: Wayland (wlr-layer-shell, bottom layer)

  clock 1.0.0  <span class="g">running</span>
    320x120 at 48,48 on DP-1
  system-load 1.2.0  <span class="g">running</span>
    300x180 at 48,184 on DP-1, locked</pre>
    </div>

    <figcaption>
      Below your windows, above your wallpaper.
      <span class="wake">clock woken {{ wakes }}× · next at the top of the minute</span>
    </figcaption>
  </figure>
</template>

<style scoped>
.uw-desk {
  position: relative;
  width: min(520px, 92vw);
  aspect-ratio: 13 / 10;
  margin: 0;
  border-radius: 22px;
  overflow: hidden;
  isolation: isolate;
  box-shadow:
    0 40px 80px -40px rgba(40, 30, 140, 0.55),
    0 0 0 1px rgba(255, 255, 255, 0.12) inset;
  font-family: var(--vp-font-family-base);
  color: #f4f2ff;
  animation: rise 0.8s cubic-bezier(0.2, 0.7, 0.2, 1) both;
}

.wall {
  position: absolute;
  inset: 0;
  z-index: -1;
  background: linear-gradient(135deg, #2a1f78 0%, #1f3fb0 45%, #0e7f86 100%);
}

.blob {
  position: absolute;
  border-radius: 50%;
  filter: blur(40px);
  opacity: 0.75;
}
.b1 { width: 60%; height: 60%; left: -10%; top: -15%; background: #7c5cff; }
.b2 { width: 55%; height: 55%; right: -10%; bottom: -10%; background: #14c8b0; }
.b3 { width: 35%; height: 35%; right: 20%; top: 10%; background: #4f7bff; }

.w {
  position: absolute;
  padding: 14px 16px;
  border-radius: 16px;
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.2), rgba(255, 255, 255, 0.07));
  border: 1px solid rgba(255, 255, 255, 0.24);
  box-shadow: 0 14px 30px -14px rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(14px) saturate(140%);
  -webkit-backdrop-filter: blur(14px) saturate(140%);
}

.k {
  font-size: 10.5px;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: rgba(244, 242, 255, 0.75);
}
.big {
  font-family: var(--uw-font-display);
  font-size: 46px;
  font-weight: 700;
  letter-spacing: -0.03em;
  line-height: 1.05;
}
.mid {
  font-family: var(--uw-font-display);
  font-size: 28px;
  font-weight: 700;
  line-height: 1.2;
}
.sub { font-size: 12px; color: rgba(244, 242, 255, 0.75); }

.clock { left: 5%; top: 6%; width: 52%; }
.load { left: 5%; top: 40%; width: 46%; }
.batt { left: 55%; top: 40%; width: 26%; }

.row { display: flex; justify-content: space-between; align-items: center; }
.chip {
  font-family: var(--vp-font-family-mono);
  font-size: 9.5px;
  padding: 2px 7px;
  border-radius: 99px;
  background: rgba(255, 181, 71, 0.22);
  color: #ffdba3;
}
.bar { height: 6px; border-radius: 3px; background: rgba(255, 255, 255, 0.18); margin: 6px 0 8px; }
.bar i { display: block; height: 100%; border-radius: 3px; background: linear-gradient(90deg, #14c8b0, #4f7bff); }
.spark { width: 100%; height: 40px; display: block; }

.win {
  position: absolute;
  right: -4%;
  top: 10%;
  width: 46%;
  border-radius: 12px;
  background: rgba(14, 12, 32, 0.92);
  border: 1px solid rgba(255, 255, 255, 0.12);
  box-shadow: 0 30px 60px -20px rgba(0, 0, 0, 0.7);
  overflow: hidden;
}
.bar-top { display: flex; gap: 5px; padding: 8px 10px; background: rgba(255, 255, 255, 0.05); }
.bar-top i { width: 8px; height: 8px; border-radius: 50%; background: rgba(255, 255, 255, 0.25); }
.win pre {
  margin: 0;
  padding: 10px 12px 12px;
  font-family: var(--vp-font-family-mono);
  font-size: 9.5px;
  line-height: 1.65;
  color: #d9d6f5;
  white-space: pre;
  overflow: hidden;
}
.p { color: #86a3ff; }
.g { color: #4fe3cf; }
.d { color: #807ca6; }

figcaption {
  position: absolute;
  left: 5%;
  bottom: 5%;
  right: 5%;
  font-size: 13px;
  font-weight: 600;
  display: flex;
  flex-direction: column;
  gap: 2px;
  text-shadow: 0 1px 8px rgba(0, 0, 0, 0.35);
}
.wake {
  font-family: var(--vp-font-family-mono);
  font-weight: 400;
  font-size: 10.5px;
  color: rgba(244, 242, 255, 0.72);
}

@keyframes rise {
  from { opacity: 0; transform: translateY(14px) scale(0.985); }
  to { opacity: 1; transform: none; }
}
@media (prefers-reduced-motion: reduce) {
  .uw-desk { animation: none; }
}
@media (max-width: 640px) {
  .uw-desk { aspect-ratio: 1 / 1; }
  .win { display: none; }
  .clock { width: 88%; }
  .big { font-size: 40px; }
  .load { top: 37%; width: 52%; }
  .batt { left: 61%; top: 37%; width: 33%; }
  .sub { display: none; }
}
</style>
