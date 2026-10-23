import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  documentation: [
    'index',
    {
      type: 'category',
      label: 'Get started',
      collapsed: false,
      items: ['Installation', 'QuickStart', 'Hyperliquid', 'Providers', 'Architecture', 'Testing'],
    },
    {
      type: 'category',
      label: 'Features',
      collapsed: false,
      items: [
        'Agents',
        'Dashboard',
        'Charts',
        'Indicators',
        'Transactions',
        'Chat',
        'SubAgents',
        'Prompts',
        'MCPTools',
        'Memory',
        'Insights',
        'Notifications',
        'Settings',
        'Telegram',
      ],
    },
  ],
};

export default sidebars;
