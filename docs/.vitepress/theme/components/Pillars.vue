<script setup lang="ts">
// The three properties the project is built around, each with the number or
// mechanism that backs it, rather than an adjective.
const pillars = [
  {
    kicker: 'Idle',
    title: 'Nothing happening costs nothing',
    stat: '0',
    unit: 'GPU submits per idle second',
    body: 'No idle loop and no idle frame. Every widget says when it wants to be called again; the host sleeps until the earliest of those, and a view that did not change draws nothing.',
  },
  {
    kicker: 'Sandbox',
    title: 'Code you did not write, contained',
    stat: '16',
    unit: 'ms deadline per call',
    body: 'Widgets are WASM components with no ambient authority: no files, no network, no clock unless the manifest asked and the user agreed — and then only for the named folder, host or command.',
  },
  {
    kicker: 'Quality',
    title: 'One look, many authors',
    stat: '21',
    unit: 'golden images in CI',
    body: 'Widgets describe roles and accents; the host owns fonts, colour and layout. Analytic anti-aliasing, Oklch colour, real shadows and shaped text — per-pixel alpha against your wallpaper.',
  },
]
</script>

<template>
  <div class="uw-pillars">
    <article v-for="p in pillars" :key="p.kicker" class="pillar">
      <div class="uw-kicker">{{ p.kicker }}</div>
      <h3 class="uw-display">{{ p.title }}</h3>
      <div class="stat"><span class="n uw-grad">{{ p.stat }}</span><span class="u">{{ p.unit }}</span></div>
      <p>{{ p.body }}</p>
    </article>
  </div>
</template>

<style scoped>
.uw-pillars {
  display: grid;
  grid-template-columns: 1fr;
  gap: 16px;
  margin-top: 28px;
}
@media (min-width: 860px) {
  .uw-pillars { grid-template-columns: repeat(3, 1fr); }
}
.pillar {
  padding: 26px 24px;
  border-radius: 18px;
  background: var(--uw-glass-bg);
  border: 1px solid var(--uw-glass-border);
  box-shadow: var(--uw-glass-shadow);
}
.pillar h3 {
  margin: 8px 0 18px;
  font-size: 20px;
  line-height: 1.3;
}
.stat { display: flex; align-items: baseline; gap: 10px; }
.n {
  font-family: var(--uw-font-display);
  font-size: 54px;
  font-weight: 700;
  line-height: 1;
  letter-spacing: -0.04em;
}
.u { font-size: 13px; color: var(--vp-c-text-2); }
.pillar p { margin: 16px 0 0; font-size: 14.5px; line-height: 1.65; color: var(--vp-c-text-2); }
</style>
