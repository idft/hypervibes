import type {ClientModule} from '@docusaurus/types';

type GoatCounter = {
  count: (options?: {path?: string}) => void;
};

declare global {
  interface Window {
    goatcounter?: GoatCounter;
  }
}

function countPage(path: string) {
  if (window.goatcounter?.count) {
    window.goatcounter.count({path});
    return;
  }

  const script = document.querySelector<HTMLScriptElement>('script[data-goatcounter]');
  script?.addEventListener(
    'load',
    () => window.goatcounter?.count({path}),
    {once: true},
  );
}

const goatCounter: ClientModule = {
  onRouteDidUpdate({location, previousLocation}) {
    const path = location.pathname + location.search;
    const previousPath = previousLocation
      ? previousLocation.pathname + previousLocation.search
      : null;

    if (path !== previousPath) {
      countPage(path);
    }
  },
};

export default goatCounter;
