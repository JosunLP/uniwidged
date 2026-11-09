import { h } from 'vue'
import DefaultTheme from 'vitepress/theme'
import type { Theme } from 'vitepress'
import HeroDesktop from './components/HeroDesktop.vue'
import CapabilityGrid from './components/CapabilityGrid.vue'
import WidgetGallery from './components/WidgetGallery.vue'
import Pillars from './components/Pillars.vue'
import './style.css'

export default {
  extends: DefaultTheme,
  Layout() {
    return h(DefaultTheme.Layout, null, {
      'home-hero-image': () => h(HeroDesktop),
    })
  },
  enhanceApp({ app }) {
    app.component('CapabilityGrid', CapabilityGrid)
    app.component('WidgetGallery', WidgetGallery)
    app.component('Pillars', Pillars)
  },
} satisfies Theme
