---
outline: [2, 2]
---

# Roadmap

<div class="uw-roadmap">
  <div class="step done"><b>0.1</b><span>Linux desktop, every capability served</span></div>
  <div class="step done"><b>1.0</b><span>ABI frozen · docs · stability promise</span></div>
  <div class="step"><b>1.x</b><span>Windows (M5) · macOS (M6)</span></div>
  <div class="step"><b>2.0</b><span>only if the interface must break</span></div>
</div>

::: info About this page
This is `ROADMAP.md` from the repository: every milestone, what it was for,
how it was measured, and what was found. The most recent decisions are in
[Towards 1.0.0](#towards-1-0-0) and [After 1.0.0](#after-1-0-0).
:::

<!--@include: ../../ROADMAP.md{2,}-->

<style>
.uw-roadmap { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; margin: 24px 0; }
.uw-roadmap .step { padding: 14px; border-radius: 14px; border: 1px dashed var(--vp-c-divider); }
.uw-roadmap .step.done { border: 1px solid var(--uw-glass-border); background: var(--uw-glass-bg); box-shadow: var(--uw-glass-shadow); }
.uw-roadmap b { display: block; font-family: var(--uw-font-display); font-size: 22px; }
.uw-roadmap .done b { background: var(--uw-gradient); -webkit-background-clip: text; background-clip: text; color: transparent; }
.uw-roadmap span { font-size: 13px; color: var(--vp-c-text-2); }
</style>
