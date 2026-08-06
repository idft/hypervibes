import "./core/htmx";
import { installCsrf, seedCsrfTokens } from "./core/csrf";
import { initAccount } from "./features/account";
import { initAgentPage, installAgentPageLifecycle } from "./features/agents/page";
import { initRunTranscripts, installAgentLiveLifecycle } from "./features/agents/live";
import { initMemoryTimelines, installMemoryLifecycle } from "./features/agents/memories";
import { initModelPickers, installModelPickerLifecycle } from "./features/agents/model-picker";
import { installAgentNavigation } from "./features/agents/navigation";
import { initProviders } from "./features/providers";
import { installCopyButtons } from "./shared/clipboard";
import { initClickableRows } from "./shared/clickable-rows";
import { renderLocalDateTimes, renderTimeago, seedNumberRolls, startRunningDurationTicker } from "./shared/presentation";

function initialize(root: ParentNode = document) {
  seedCsrfTokens(root);
  renderTimeago(root);
  renderLocalDateTimes(root);
  seedNumberRolls(root);
  initClickableRows(root);
  initAccount(root);
  initAgentPage(root);
  initMemoryTimelines(root);
  initModelPickers(root);
  initProviders(root);
  initRunTranscripts(root);
}

function start() {
  installCsrf();
  installCopyButtons();
  installAgentNavigation();
  installAgentPageLifecycle();
  installAgentLiveLifecycle();
  installMemoryLifecycle();
  installModelPickerLifecycle();
  document.addEventListener("htmx:afterSwap", (event) => {
    const swappedElement = (event as CustomEvent<{ elt?: unknown }>).detail.elt;
    if (swappedElement instanceof Element) {
      initialize(swappedElement);
    }
  });
  initialize();
  startRunningDurationTicker();
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", start, { once: true });
} else {
  start();
}
