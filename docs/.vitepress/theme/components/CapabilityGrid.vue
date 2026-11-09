<script setup lang="ts">
const caps = [
  { name: 'timer', grants: 'Being woken again — any Wake other than Never.', ask: 'Ask when the view changes with time.' },
  { name: 'locale', grants: 'Dates and times formatted in the user’s locale and time zone.', ask: 'Clocks and calendars.' },
  { name: 'measure', grants: 'Measuring your own text, in a named role, with the host’s real fonts.', ask: 'Fitting text before you show it.' },
  { name: 'metrics', grants: 'CPU, memory, disk, battery and network counters — one grant for “how is the machine doing”.', ask: 'System monitors.' },
  { name: 'fs:<subpath>', grants: 'Reading and writing one folder inside the widget’s own directory. Symlinks cannot leave it.', ask: 'Caches and saved state.' },
  { name: 'net:<host>', grants: 'HTTPS to exactly that host. A redirect off it is refused.', ask: 'One API, by name.' },
  { name: 'link:<host>', grants: 'Opening https pages on that host in the browser — at the user’s click, never on its own.', ask: '“Open forecast” buttons.' },
  { name: 'secrets', grants: 'An encrypted keychain namespaced to the widget. It cannot name another’s.', ask: 'API tokens.' },
  { name: 'command:<name>', grants: 'Running one declared external command, asynchronously, with no shell.', ask: 'Wrapping a CLI tool.' },
]
</script>

<template>
  <div class="uw-caps">
    <div v-for="c in caps" :key="c.name" class="cap">
      <code class="n">{{ c.name }}</code>
      <p>{{ c.grants }}</p>
      <div class="ask">{{ c.ask }}</div>
    </div>
  </div>
</template>

<style scoped>
.uw-caps {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 12px;
  margin: 24px 0;
}
.cap {
  padding: 16px;
  border-radius: 14px;
  background: var(--uw-glass-bg);
  border: 1px solid var(--uw-glass-border);
  box-shadow: var(--uw-glass-shadow);
}
.n { font-size: 13px; font-weight: 600; }
.cap p { margin: 10px 0 8px; font-size: 13.5px; line-height: 1.55; color: var(--vp-c-text-2); }
.ask { font-size: 12px; color: var(--vp-c-text-3); }
</style>
