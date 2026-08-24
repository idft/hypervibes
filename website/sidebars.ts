import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  documentation: [
    'index',
    'QuickStart',
    {
      type: 'category',
      label: 'Operation',
      items: ['Installation', 'Hyperliquid', 'Providers', 'MCPTools'],
    },
    {
      type: 'category',
      label: 'Concepts',
      items: ['Agents', 'Prompts', 'SubAgents', 'Chat', 'Memory'],
    },
    {
      type: 'category',
      label: 'Development',
      items: ['Architecture', 'Testing'],
    },
  ],
};

export default sidebars;
