import "./core/htmx";
import { installCsrf, seedCsrfTokens } from "./core/csrf";
import { initAccount } from "./features/account";
import { initAgentPage, installAgentPageLifecycle } from "./features/agents/page";
import { initRunTranscripts, installAgentLiveLifecycle } from "./features/agents/live";
import { initMemoryTimelines, installMemoryLifecycle } from "./features/agents/memories";
import { initModelPickers, installModelPickerLifecycle } from "./features/agents/model-picker";
import { initSingletonRoleForms } from "./features/agents/singleton-role-form";
import { installAgentNavigation } from "./features/agents/navigation";
import { initIndicators, installIndicatorsLifecycle } from "./features/agents/indicators";
import { initProviders } from "./features/providers";
import { installCopyButtons } from "./shared/clipboard";
import { initClickableRows } from "./shared/clickable-rows";
import { animateNumberRolls, renderLocalDateTimes, renderTimeago, seedNumberRolls, startRunningDurationTicker } from "./shared/presentation";

function initialize(root: ParentNode = document, animateNumbers = false) {
  seedCsrfTokens(root);
  renderTimeago(root);
  renderLocalDateTimes(root);
  // Live values must be compared before seeding overwrites the previous value.
  if (animateNumbers) animateNumberRolls(root);
  else seedNumberRolls(root);
  initClickableRows(root);
  initAccount(root);
  initAgentPage(root);
  initIndicators(root);
  initMemoryTimelines(root);
  initModelPickers(root);
  initSingletonRoleForms(root);
  initProviders(root);
  initRunTranscripts(root);
}

function start() {
  installCsrf();
  installCopyButtons();
  installAgentNavigation();
  installIndicatorsLifecycle();
  installAgentPageLifecycle();
  installAgentLiveLifecycle();
  installMemoryLifecycle();
  installModelPickerLifecycle();
  document.addEventListener("htmx:afterSwap", (event) => {
    const swappedElement = (event as CustomEvent<{ elt?: unknown }>).detail.elt;
    if (swappedElement instanceof Element) {
      initialize(swappedElement, swappedElement.matches('[sse-swap="balance"]'));
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
