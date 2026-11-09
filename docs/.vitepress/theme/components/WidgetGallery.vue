<script setup lang="ts">
const repo = 'https://github.com/JosunLP/uniwidged/tree/main/widgets/'

// One per capability — and, for `metrics`, one per kind of answer it gives.
const widgets = [
  { name: 'clock', caps: ['timer', 'locale'], body: 'The time and the date, in your locale. Woken at the top of each minute — the smallest useful widget there is.', teaches: 'Wake::At, locale formatting' },
  { name: 'system-load', caps: ['timer', 'metrics'], body: 'CPU, memory and disk, with a minute of history under the meter and a warning threshold you can configure.', teaches: 'metrics, sparkline, settings' },
  { name: 'battery', caps: ['timer', 'metrics'], body: 'Charge, time left and an hour of history — and “on mains power” on a desktop that has no battery at all.', teaches: 'Ok(None) as a normal answer' },
  { name: 'network', caps: ['timer', 'metrics'], body: 'Down and up, a minute of each on one shared scale.', teaches: 'scaling a rate for a chart' },
  { name: 'counter', caps: ['fs:state'], body: 'Counts clicks and remembers the count across a crash, a restart and a reboot.', teaches: 'storage, state derived from data' },
  { name: 'uptime', caps: ['timer', 'command:uptime'], body: 'How long this machine has been up, by asking a declared command — asynchronously.', teaches: 'run_command, take_result' },
  { name: 'vault', caps: ['secrets'], body: 'Keeps a credential in the keychain rather than a file, and never lets it reach the view.', teaches: 'secrets, forgetting' },
  { name: 'conformance', caps: [], body: 'Tries every capability under a deny-all policy so the sandbox can be tested for what it refuses.', teaches: 'what the sandbox says no to' },
]
</script>

<template>
  <div class="uw-gallery">
    <a v-for="w in widgets" :key="w.name" class="card" :href="repo + w.name" target="_blank" rel="noopener">
      <div class="head">
        <span class="name">{{ w.name }}</span>
        <span class="arrow" aria-hidden="true">↗</span>
      </div>
      <p>{{ w.body }}</p>
      <div class="caps">
        <code v-for="c in w.caps" :key="c">{{ c }}</code>
        <code v-if="!w.caps.length" class="none">no capabilities</code>
      </div>
      <div class="teaches">Teaches: {{ w.teaches }}</div>
    </a>
  </div>
</template>

<style scoped>
.uw-gallery {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
  gap: 14px;
  margin: 24px 0;
}
.card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 18px;
  border-radius: 16px;
  background: var(--uw-glass-bg);
  border: 1px solid var(--uw-glass-border);
  box-shadow: var(--uw-glass-shadow);
  text-decoration: none !important;
  color: inherit !important;
  transition: transform 0.2s ease, border-color 0.2s ease;
}
.card:hover { transform: translateY(-2px); border-color: rgba(79, 123, 255, 0.45); }
.head { display: flex; justify-content: space-between; align-items: center; }
.name { font-family: var(--uw-font-display); font-weight: 700; font-size: 17px; }
.arrow { color: var(--vp-c-brand-1); }
.card p { margin: 0; font-size: 14px; line-height: 1.55; color: var(--vp-c-text-2); flex: 1; }
.caps { display: flex; flex-wrap: wrap; gap: 6px; }
.caps code { font-size: 11.5px; }
.caps .none { opacity: 0.7; }
.teaches { font-size: 12px; color: var(--vp-c-text-3); }
</style>
