import type {ReactNode} from 'react';
import Layout from '@theme-original/Layout';
import {TitleFormatterProvider} from '@docusaurus/theme-common/internal';

const formatTitle: Parameters<typeof TitleFormatterProvider>[0]['formatter'] = ({
  title,
  siteTitle,
  titleDelimiter,
}) => {
  const trimmedTitle = title?.trim();

  if (!trimmedTitle || trimmedTitle === siteTitle) {
    return siteTitle;
  }

  return `${siteTitle} ${titleDelimiter} ${trimmedTitle}`;
};

export default function LayoutWrapper(props: {children: ReactNode}) {
  return (
    <TitleFormatterProvider formatter={formatTitle}>
      <Layout {...props} />
    </TitleFormatterProvider>
  );
}
