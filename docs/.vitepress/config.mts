import { defineConfig } from 'vitepress'

const repo = 'https://github.com/JosunLP/uniwidged'
const base = process.env.DOCS_BASE ?? '/uniwidged/'

export default defineConfig({
  lang: 'en-GB',
  title: 'uniwidged',
  titleTemplate: ':title · uniwidged',
  description:
    'Sandboxed, GPU-rendered desktop widgets that cost nothing when nothing is happening. Widgets are WASM components; the host draws them.',
  base,
  cleanUrls: true,
  lastUpdated: true,
  appearance: true,

  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: `${base}favicon.svg` }],
    ['link', { rel: 'apple-touch-icon', href: `${base}apple-touch-icon.png` }],
    ['meta', { name: 'theme-color', content: '#4F7BFF' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:site_name', content: 'uniwidged' }],
    ['meta', { property: 'og:image', content: `https://josunlp.github.io${base}og-image.png` }],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }],
    ['link', { rel: 'preconnect', href: 'https://fonts.googleapis.com' }],
    ['link', { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: '' }],
    [
      'link',
      {
        rel: 'stylesheet',
        href: 'https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@500;600;700&family=JetBrains+Mono:wght@400;600&display=swap',
      },
    ],
  ],

  markdown: {
    theme: { light: 'github-light', dark: 'one-dark-pro' },
    lineNumbers: false,
  },

  sitemap: { hostname: `https://josunlp.github.io${base}` },

  themeConfig: {
    logo: { src: '/logo.svg', alt: '' },
    siteTitle: 'uniwidged',

    nav: [
      { text: 'Guide', link: '/guide/introduction', activeMatch: '/guide/' },
      { text: 'Reference', link: '/reference/api', activeMatch: '/reference/' },
      { text: 'Widgets', link: '/guide/gallery' },
      {
        text: 'v1.0.0',
        items: [
          { text: 'Changelog', link: '/project/changelog' },
          { text: 'Roadmap', link: '/project/roadmap' },
          { text: 'Stability promise', link: '/reference/abi' },
          { text: 'Releases', link: `${repo}/releases` },
        ],
      },
    ],

    sidebar: {
      '/guide/': sidebarGuide(),
      '/reference/': sidebarReference(),
      '/project/': sidebarProject(),
    },

    outline: { level: [2, 3], label: 'On this page' },

    socialLinks: [{ icon: 'github', link: repo }],

    editLink: {
      pattern: `${repo}/edit/main/docs/:path`,
      text: 'Improve this page on GitHub',
    },

    search: {
      provider: 'local',
      options: { detailedView: true },
    },

    footer: {
      message: 'Released under the MIT or Apache-2.0 licence, at your option.',
      copyright: 'Copyright © 2026 the uniwidged contributors',
    },

    docFooter: { prev: 'Previous', next: 'Next' },
    externalLinkIcon: true,
  },
})

function sidebarGuide() {
  return [
    {
      text: 'Getting started',
      items: [
        { text: 'Introduction', link: '/guide/introduction' },
        { text: 'Installation', link: '/guide/installation' },
        { text: 'Quick start', link: '/guide/quick-start' },
        { text: 'Core concepts', link: '/guide/concepts' },
      ],
    },
    {
      text: 'Using uniwidged',
      items: [
        { text: 'Running the desktop', link: '/guide/desktop' },
        { text: 'Finding widgets', link: '/guide/registries' },
        { text: 'Reference widgets', link: '/guide/gallery' },
        { text: 'Troubleshooting', link: '/guide/troubleshooting' },
      ],
    },
    {
      text: 'Building widgets',
      items: [
        { text: 'Writing a widget', link: '/guide/writing-a-widget' },
        { text: 'Capabilities', link: '/guide/capabilities' },
      ],
    },
    { text: 'Reference →', link: '/reference/api' },
  ]
}

function sidebarReference() {
  return [
    {
      text: 'Reference',
      items: [
        { text: 'Widget API', link: '/reference/api' },
        { text: 'Command line', link: '/reference/cli' },
        { text: 'Configuration files', link: '/reference/configuration' },
        { text: 'ABI & stability', link: '/reference/abi' },
      ],
    },
    { text: '← Guide', link: '/guide/introduction' },
  ]
}

function sidebarProject() {
  return [
    {
      text: 'Project',
      items: [
        { text: 'Architecture', link: '/project/architecture' },
        { text: 'Security model', link: '/project/security' },
        { text: 'Roadmap', link: '/project/roadmap' },
        { text: 'Changelog', link: '/project/changelog' },
        { text: 'Contributing', link: '/project/contributing' },
        { text: 'FAQ', link: '/project/faq' },
        { text: 'Brand', link: '/project/brand' },
      ],
    },
  ]
}
