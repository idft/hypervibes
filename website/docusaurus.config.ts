import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'HyperVibes',
  tagline: 'OpenCode-backed trading agents for Hyperliquid',
  favicon: 'img/favicon.png',
  url: 'https://hypervibes.ai',
  baseUrl: '/',
  organizationName: 'idft',
  projectName: 'hypervibes',
  onBrokenLinks: 'throw',
  clientModules: ['./src/clientModules/goatcounter.ts'],
  headTags: [
    {
      tagName: 'link',
      attributes: {rel: 'preconnect', href: 'https://fonts.googleapis.com'},
    },
    {
      tagName: 'link',
      attributes: {
        rel: 'preconnect',
        href: 'https://fonts.gstatic.com',
        crossOrigin: 'anonymous',
      },
    },
    {
      tagName: 'link',
      attributes: {
        rel: 'stylesheet',
        href: 'https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500;600&family=IBM+Plex+Sans:wght@400;500;600;700&display=swap',
      },
    },
    {
      tagName: 'script',
      attributes: {
        'data-goatcounter': 'https://hypervibes.goatcounter.com/count',
        'data-goatcounter-settings': JSON.stringify({no_onload: true}),
        async: true,
        src: '//gc.zgo.at/count.js',
      },
    },
  ],
  markdown: {
    hooks: {
      onBrokenMarkdownLinks: 'warn',
    },
  },
  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },
  presets: [
    [
      'classic',
      {
        docs: {
          path: '../docs',
          routeBasePath: 'docs',
          sidebarPath: './sidebars.ts',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],
  themeConfig: {
    colorMode: {
      defaultMode: 'dark',
      disableSwitch: true,
      respectPrefersColorScheme: false,
    },
    navbar: {
      title: 'HyperVibes',
      items: [
        {to: '/docs/', label: 'Documentation', position: 'left'},
        {
          href: 'https://github.com/idft/hypervibes',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      copyright: `Copyright ${new Date().getFullYear()} HyperVibes.`,
    },
    prism: {
      theme: {
        plain: {
          color: '#e4e4e7',
          backgroundColor: '#0d0d10',
        },
        styles: [],
      },
      darkTheme: {
        plain: {
          color: '#e4e4e7',
          backgroundColor: '#0d0d10',
        },
        styles: [],
      },
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
