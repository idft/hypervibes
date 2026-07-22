import * as htmx from "htmx.org";
import { createWalletClient, custom } from "viem";
(window as unknown as { htmx: typeof htmx }).htmx = htmx;
import "htmx-ext-sse";

function csrfToken(): string | undefined {
  return document.cookie.split("; ").find((cookie) => cookie.startsWith("vt_csrf="))?.split("=", 2)[1];
}

document.addEventListener("htmx:configRequest", (event) => {
  const token = csrfToken();
  if (token) (event as CustomEvent).detail.headers["X-CSRF-Token"] = token;
});

document.addEventListener("submit", (event) => {
  const form = event.target;
  const token = csrfToken();
  if (!(form instanceof HTMLFormElement) || !token || form.method.toLowerCase() === "get") return;
  let input = form.querySelector<HTMLInputElement>('input[name="csrf_token"]');
  if (!input) {
    input = document.createElement("input");
    input.type = "hidden";
    input.name = "csrf_token";
    form.append(input);
  }
  input.value = token;
}, true);

import { render } from "timeago.js";

function renderTimeago(root: ParentNode = document) {
  const nodes = root.querySelectorAll("time.timeago");
  if (nodes.length > 0) {
    render(nodes);
    nodes.forEach((node) => node.classList.add("timeago-ready"));
  }
}

type LocalDateTimeFormat = "date" | "time" | "datetime";

function formatLocalDateTime(date: Date, format: LocalDateTimeFormat): string {
  try {
    switch (format) {
      case "date":
        return new Intl.DateTimeFormat(undefined, {
          year: "numeric",
          month: "short",
          day: "numeric",
        }).format(date);
      case "time":
        return new Intl.DateTimeFormat(undefined, {
          hour: "numeric",
          minute: "2-digit",
        }).format(date);
      default:
        return new Intl.DateTimeFormat(undefined, {
          year: "numeric",
          month: "short",
          day: "numeric",
          hour: "numeric",
          minute: "2-digit",
        }).format(date);
    }
  } catch {
    switch (format) {
      case "date":
        return date.toLocaleDateString();
      case "time":
        return date.toLocaleTimeString();
      default:
        return date.toLocaleString();
    }
  }
}

function renderLocalDateTimeNode(node: Element) {
  const datetime = node.getAttribute("datetime");
  if (!datetime) {
    return;
  }

  const date = new Date(datetime);
  if (Number.isNaN(date.getTime())) {
    return;
  }

  const format =
    (node.getAttribute("data-local-format") as LocalDateTimeFormat | null) ??
    "datetime";
  node.textContent = formatLocalDateTime(date, format);
}

function renderLocalDateTimes(root: ParentNode = document) {
  const nodes = root.querySelectorAll("time.local-datetime");
  nodes.forEach((node) => {
    renderLocalDateTimeNode(node);
  });
}

interface NumberState {
  raw: string;
  formatted: string;
}

const previousValues = new Map<string, NumberState>();

const rollUpKeyframes: Keyframe[] = [
  { transform: "translateY(-0.4em)", opacity: "0", color: "#4ade80" },
  { transform: "translateY(0)", opacity: "1", color: "#4ade80", offset: 0.85 },
  { transform: "translateY(0)", opacity: "1", color: "inherit" },
];

const rollDownKeyframes: Keyframe[] = [
  { transform: "translateY(0.4em)", opacity: "0", color: "#f87171" },
  { transform: "translateY(0)", opacity: "1", color: "#f87171", offset: 0.85 },
  { transform: "translateY(0)", opacity: "1", color: "inherit" },
];

const rollTiming: KeyframeAnimationOptions = {
  duration: 1500,
  easing: "cubic-bezier(0.22, 0.61, 0.36, 1)",
  fill: "forwards",
};

function readFormattedDigits(container: HTMLElement): string {
  const digits = container.querySelectorAll<HTMLElement>(".number-digit");
  return Array.from(digits, (d) => d.getAttribute("data-digit") ?? "").join("");
}

function animateNumberRoll(container: HTMLElement) {
  const key = container.getAttribute("data-animate-key");
  if (!key) return;

  const digits = container.querySelectorAll<HTMLElement>(".number-digit");
  if (digits.length === 0) return;

  const rawValue = container.getAttribute("data-raw-value") ?? "";
  const formattedValue = readFormattedDigits(container);

  const prev = previousValues.get(key);

  if (prev && prev.formatted !== formattedValue) {
    const rawPrev = parseFloat(prev.raw);
    const rawNext = parseFloat(rawValue);
    const up = !isNaN(rawPrev) && !isNaN(rawNext) && rawNext > rawPrev;
    const keyframes = up ? rollUpKeyframes : rollDownKeyframes;

    let firstChanged = -1;
    for (let i = 0; i < digits.length; i++) {
      const digit = digits[i].getAttribute("data-digit") ?? "";
      if (i >= prev.formatted.length || prev.formatted[i] !== digit) {
        firstChanged = i;
        break;
      }
    }

    if (firstChanged >= 0) {
      let stagger = 0;
      for (let i = firstChanged; i < digits.length; i++) {
        digits[i].animate(keyframes, { ...rollTiming, delay: stagger });
        stagger += 60;
      }
    }
  }

  previousValues.set(key, { raw: rawValue, formatted: formattedValue });
}

function seedNumberRoll(container: HTMLElement) {
  const key = container.getAttribute("data-animate-key");
  if (!key) return;
  const rawValue = container.getAttribute("data-raw-value") ?? "";
  const formattedValue = readFormattedDigits(container);
  previousValues.set(key, { raw: rawValue, formatted: formattedValue });
}

function formatElapsedDuration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  if (total < 60) {
    return `${total}s`;
  }
  if (total < 3600) {
    const minutes = Math.floor(total / 60);
    const remSeconds = total % 60;
    if (remSeconds === 0) {
      return `${minutes}m`;
    }
    return `${minutes}m ${remSeconds}s`;
  }
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  if (minutes === 0) {
    return `${hours}h`;
  }
  return `${hours}h ${minutes}m`;
}

function tickRunningDurations() {
  const now = Date.now();
  document
    .querySelectorAll<HTMLElement>("[data-running-duration]")
    .forEach((node) => {
      const startedAt = node.getAttribute("data-started-at");
      if (!startedAt) {
        return;
      }
      const startedMs = new Date(startedAt).getTime();
      if (Number.isNaN(startedMs)) {
        return;
      }
      const elapsedSeconds = (now - startedMs) / 1000;
      node.textContent = formatElapsedDuration(elapsedSeconds);
    });
}

function startRunningDurationTicker() {
  tickRunningDurations();
  window.setInterval(tickRunningDurations, 1000);
}

function setActiveMemoryTimelineItem(activeItem: HTMLElement) {
  const root = activeItem.closest<HTMLElement>("[data-agent-memories]");
  if (root) {
    root.dataset.selectedMemoryId = activeItem.dataset.memoryId ?? "";
  }

  const scope = root ?? document;
  scope
    .querySelectorAll<HTMLElement>("[data-memory-timeline-item]")
    .forEach((item) => {
      item.setAttribute("aria-pressed", item === activeItem ? "true" : "false");
    });
}

function restoreSelectedMemoryTimelineItem(root: ParentNode = document) {
  const memoryRoots =
    root instanceof Element && root.matches("[data-agent-memories]")
      ? [root as HTMLElement]
      : Array.from(root.querySelectorAll<HTMLElement>("[data-agent-memories]"));

  memoryRoots.forEach((memoryRoot) => {
    const detail = memoryRoot.querySelector<HTMLElement>("#memory-detail[data-memory-detail-id]");
    const selectedMemoryId = detail?.dataset.memoryDetailId ?? memoryRoot.dataset.selectedMemoryId;
    if (!selectedMemoryId) {
      return;
    }

    memoryRoot.dataset.selectedMemoryId = selectedMemoryId;

    let matched = false;
    memoryRoot
      .querySelectorAll<HTMLElement>("[data-memory-timeline-item]")
      .forEach((item) => {
        const selected = item.dataset.memoryId === selectedMemoryId;
        matched ||= selected;
        item.setAttribute(
          "aria-pressed",
          selected ? "true" : "false",
        );
      });
    console.debug("memory timeline restore selection", {
      selectedMemoryId,
      matched,
    });
  });
}

function seedSelectedMemoryTimelineItems(root: ParentNode = document) {
  const memoryRoots =
    root instanceof Element && root.matches("[data-agent-memories]")
      ? [root as HTMLElement]
      : Array.from(root.querySelectorAll<HTMLElement>("[data-agent-memories]"));

  memoryRoots.forEach((memoryRoot) => {
    if (memoryRoot.dataset.selectedMemoryId) {
      return;
    }
    const selectedItem = memoryRoot.querySelector<HTMLElement>(
      '[data-memory-timeline-item][aria-pressed="true"]',
    );
    if (selectedItem?.dataset.memoryId) {
      memoryRoot.dataset.selectedMemoryId = selectedItem.dataset.memoryId;
    }
  });
}

let memoryDetailAbortController: AbortController | null = null;

function loadMemoryTimelineItem(item: HTMLElement) {
  const url = item.getAttribute("hx-get");
  const target = item.getAttribute("hx-target");
  if (!url || !target) {
    return;
  }

  const targetElement = document.querySelector(target);
  if (!(targetElement instanceof HTMLElement)) {
    return;
  }

  memoryDetailAbortController?.abort();
  const abortController = new AbortController();
  memoryDetailAbortController = abortController;
  const memoryRoot = item.closest<HTMLElement>("[data-agent-memories]");
  const loadingIndicator = memoryRoot?.querySelector<HTMLElement>(
    "[data-memory-detail-loading]",
  );

  setActiveMemoryTimelineItem(item);
  item.setAttribute("aria-busy", "true");
  targetElement.setAttribute("aria-busy", "true");
  targetElement.classList.add("opacity-50");
  loadingIndicator?.classList.remove("hidden");
  loadingIndicator?.classList.add("flex");

  void fetch(url, {
    signal: abortController.signal,
    headers: {
      "HX-Request": "true",
    },
  })
    .then(async (response) => {
      if (memoryDetailAbortController !== abortController) {
        return;
      }
      if (!response.ok) {
        throw new Error(`Failed to load memory detail: ${response.status}`);
      }
      const html = await response.text();
      targetElement.outerHTML = html;
    })
    .catch((error) => {
      if (!(error instanceof DOMException && error.name === "AbortError")) {
        console.error(error);
      }
    })
    .finally(() => {
      item.removeAttribute("aria-busy");
      if (memoryDetailAbortController !== abortController) {
        return;
      }
      memoryDetailAbortController = null;
      targetElement.removeAttribute("aria-busy");
      targetElement.classList.remove("opacity-50");
      loadingIndicator?.classList.add("hidden");
      loadingIndicator?.classList.remove("flex");
    });
}

function initMemoryTimelineDragScroll() {
  document
    .querySelectorAll<HTMLElement>(".memory-timeline-scroll")
    .forEach((container) => {
      if (container.dataset.dragScrollBound === "true") {
        return;
      }
      container.dataset.dragScrollBound = "true";

      let mouseDown = false;
      let startX = 0;
      let startScrollLeft = 0;
      let dragged = false;
      let pressedItem: HTMLElement | null = null;
      let suppressClickFor: HTMLElement | null = null;
      let cleanupListeners: (() => void) | null = null;

      const stopDragging = () => {
        mouseDown = false;
        pressedItem = null;
        cleanupListeners?.();
        cleanupListeners = null;
        container.dataset.dragging = "false";
      };

      container.addEventListener("mousedown", (event: MouseEvent) => {
        if (event.button !== 0) {
          return;
        }

        mouseDown = true;
        startX = event.clientX;
        startScrollLeft = container.scrollLeft;
        dragged = false;
        pressedItem = (event.target as Element | null)?.closest<HTMLElement>(
          "[data-memory-timeline-item]",
        ) ?? null;
        container.dataset.dragging = "false";

        const handleMouseMove = (moveEvent: MouseEvent) => {
          if (!mouseDown) {
            return;
          }

          const deltaX = moveEvent.clientX - startX;
          if (!dragged && Math.abs(deltaX) > 6) {
            dragged = true;
            container.dataset.dragging = "true";
          }
          if (!dragged) {
            return;
          }

          moveEvent.preventDefault();
          container.scrollLeft = startScrollLeft - deltaX;
        };

        const handleMouseUp = () => {
          if (!mouseDown) {
            return;
          }

          if (!dragged && pressedItem) {
            suppressClickFor = pressedItem;
            loadMemoryTimelineItem(pressedItem);
            window.setTimeout(() => {
              suppressClickFor = null;
            }, 0);
          }
          stopDragging();
        };

        const handleWindowBlur = () => {
          stopDragging();
        };

        window.addEventListener("mousemove", handleMouseMove, { passive: false });
        window.addEventListener("mouseup", handleMouseUp);
        window.addEventListener("blur", handleWindowBlur);
        cleanupListeners = () => {
          window.removeEventListener("mousemove", handleMouseMove);
          window.removeEventListener("mouseup", handleMouseUp);
          window.removeEventListener("blur", handleWindowBlur);
        };
      });

      container.addEventListener(
        "click",
        (event) => {
          const item = (event.target as Element | null)?.closest<HTMLElement>(
            "[data-memory-timeline-item]",
          );
          if (!item) {
            return;
          }

          if (suppressClickFor === item) {
            event.preventDefault();
            event.stopPropagation();
            suppressClickFor = null;
            return;
          }

          if (dragged) {
            event.preventDefault();
            event.stopPropagation();
            dragged = false;
            return;
          }

          event.preventDefault();
          event.stopPropagation();
          loadMemoryTimelineItem(item);
        },
        true,
      );
    });
}

function isAgentDetailPath(pathname: string) {
  return pathname.startsWith("/agents/") && pathname !== "/agents/new";
}

function initAgentRailTransition() {
  const rail = document.querySelector<HTMLElement>(".agent-rail");
  const destination = window.sessionStorage.getItem("agent-rail-enter-destination");
  window.sessionStorage.removeItem("agent-rail-enter-destination");
  if (!rail || destination !== window.location.pathname) {
    return;
  }

  rail.classList.add("agent-rail-enter");
}

function initAgentRailEntryNavigation() {
  document.addEventListener("click", (event) => {
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    ) {
      return;
    }

    const link = (event.target as Element | null)?.closest<HTMLAnchorElement>("a[href]");
    if (!link || link.hasAttribute("hx-get")) {
      return;
    }

    const destination = new URL(link.href, window.location.href);
    if (
      destination.origin === window.location.origin &&
      isAgentDetailPath(destination.pathname) &&
      !isAgentDetailPath(window.location.pathname)
    ) {
      window.sessionStorage.setItem("agent-rail-enter-destination", destination.pathname);
    }
  });
}

function initAgentRailNavigation() {
  document.addEventListener("click", (event) => {
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    ) {
      return;
    }

    const link = (event.target as Element | null)?.closest<HTMLAnchorElement>("a[href]");
    const rail = document.querySelector<HTMLElement>(".agent-rail");
    if (!link || !rail || link.hasAttribute("hx-get")) {
      return;
    }

    const destination = new URL(link.href, window.location.href);
    const isAgentDetail =
      destination.origin === window.location.origin && isAgentDetailPath(destination.pathname);
    if (isAgentDetail || destination.origin !== window.location.origin) {
      return;
    }

    event.preventDefault();
    rail.classList.add("agent-rail-exit");
    window.setTimeout(() => {
      window.location.assign(destination.href);
    }, 180);
  });
}

function syncAgentSelector() {
  const currentAgent = document.querySelector<HTMLElement>("[data-current-agent-key]");
  const match = window.location.pathname.match(/^\/agents\/([^/]+)/);
  const agentKey = currentAgent?.dataset.currentAgentKey ?? match?.[1];
  const selected = currentAgent ?? Array.from(
    document.querySelectorAll<HTMLElement>("[data-agent-key]"),
  ).find((agent) => agent.dataset.agentKey === agentKey);
  const label = document.querySelector<HTMLElement>("[data-agent-selector-label]");
  const status = document.querySelector<HTMLElement>("[data-agent-selector-status]");
  if (!selected || !label || !status) {
    return;
  }

  label.textContent = selected.dataset.currentAgentName ?? selected.dataset.agentName ?? agentKey ?? "";
  status.classList.remove("hidden");
  status.classList.remove("bg-zinc-600", "bg-emerald-400", "bg-red-400");
  status.classList.add(
    (selected.dataset.currentAgentEnabled ?? selected.dataset.agentEnabled) === "true"
      ? "bg-emerald-400"
      : "bg-red-400",
  );
}

function initAgentSelectorDismissal() {
  document.addEventListener("click", (event) => {
    const selector = document.querySelector<HTMLDetailsElement>("[data-agent-selector]");
    if (selector?.open && !selector.contains(event.target as Node)) {
      selector.open = false;
    }
  });
}

async function copyToClipboard(value: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
      return true;
    }
  } catch {
    // Fall back for HTTP deployments where the Clipboard API is unavailable.
  }

  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.append(textarea);
  textarea.select();
  textarea.setSelectionRange(0, value.length);

  try {
    return document.execCommand("copy");
  } finally {
    textarea.remove();
  }
}

function updateCopyButton(button: HTMLButtonElement, copied: boolean) {
  const label = button.dataset.copyLabel ?? button.ariaLabel ?? "Copy";
  button.dataset.copyLabel = label;
  button.classList.remove("text-zinc-400", "text-emerald-300", "text-red-300");
  button.classList.add(copied ? "text-emerald-300" : "text-red-300");
  button.ariaLabel = copied ? "Copied" : "Copy failed";
  button.title = button.ariaLabel;

  window.setTimeout(() => {
    button.classList.remove("text-emerald-300", "text-red-300");
    button.classList.add("text-zinc-400");
    button.ariaLabel = label;
    button.title = label;
  }, 1500);
}

function initCopyButtons() {
  document.addEventListener("click", (event) => {
    const button = (event.target as Element | null)?.closest<HTMLButtonElement>(
      "[data-copy-button]",
    );
    const value = button?.dataset.copyValue;
    if (!button || !value) {
      return;
    }

    void copyToClipboard(value).then((copied) => updateCopyButton(button, copied));
  });
}

// Delegated wiring for agent_delete_modal.html. The modal markup is included
// on full pages (job/hook detail) and inside the memory detail partial, which
// is swapped in dynamically — so listeners must live on `document`, not on the
// included nodes (inline scripts do not run for swapped-in content).
function initDetailDeleteModal() {
  const closeModal = (modal: HTMLElement) => {
    modal.classList.add("hidden");
    modal.classList.remove("flex");
  };

  document.addEventListener("click", (event) => {
    const target = event.target as Element | null;
    if (!target) {
      return;
    }

    const trigger = target.closest<HTMLElement>("[data-detail-delete-trigger]");
    if (trigger) {
      const modal = document.getElementById("detail-delete-modal");
      const form = document.getElementById(
        "detail-delete-form",
      ) as HTMLFormElement | null;
      const title = document.getElementById("detail-delete-modal-title");
      const body = document.getElementById("detail-delete-modal-body");
      const confirm = document.getElementById("confirm-detail-delete-btn");
      if (!modal || !form || !title || !body || !confirm) {
        return;
      }
      const kind = trigger.dataset.deleteKind || "job";
      const label = trigger.dataset.deleteLabel || kind;
      form.action = trigger.dataset.deleteAction || "";
      title.textContent = `Delete ${kind}`;
      confirm.textContent = `Delete ${kind}`;
      body.textContent = `Are you sure you want to delete ${label}? This action cannot be undone.`;
      modal.classList.remove("hidden");
      modal.classList.add("flex");
      return;
    }

    if (target.closest("#cancel-detail-delete-btn")) {
      const modal = document.getElementById("detail-delete-modal");
      if (modal) {
        closeModal(modal);
      }
      return;
    }

    // Backdrop click: the modal root is the click target itself.
    if (target.id === "detail-delete-modal" && target instanceof HTMLElement) {
      closeModal(target);
    }
  });

  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") {
      return;
    }
    const modal = document.getElementById("detail-delete-modal");
    if (modal?.classList.contains("flex")) {
      closeModal(modal);
    }
  });
}

const HYPERLIQUID_SIGNATURE_CHAIN_ID = 42161;
const HYPERLIQUID_SIGNATURE_CHAIN_HEX = "0xa4b1";

function walletClient() {
  const provider = (window as Window & { ethereum?: Parameters<typeof custom>[0] }).ethereum;
  if (!provider) throw new Error("No Ethereum wallet found.");
  return { client: createWalletClient({ transport: custom(provider) }), provider };
}

async function selectHyperliquidSignatureChain(provider: Parameters<typeof custom>[0]) {
  try {
    await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: HYPERLIQUID_SIGNATURE_CHAIN_HEX }] });
  } catch (error: unknown) {
    if (!(typeof error === "object" && error !== null && "code" in error && error.code === 4902)) {
      throw new Error("Switch your wallet to Arbitrum Mainnet to sign Hyperliquid approvals.", { cause: error });
    }
    await provider.request({
      method: "wallet_addEthereumChain",
      params: [{
        chainId: HYPERLIQUID_SIGNATURE_CHAIN_HEX,
        chainName: "Arbitrum One",
        nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
        rpcUrls: ["https://arb1.arbitrum.io/rpc"],
        blockExplorerUrls: ["https://arbiscan.io"],
      }],
    });
    await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: HYPERLIQUID_SIGNATURE_CHAIN_HEX }] });
  }
}

async function signWalletAction(primaryType: string, types: Record<string, readonly { name: string; type: string }[]>, action: Record<string, string | number | boolean>) {
  const { client, provider } = walletClient();
  await selectHyperliquidSignatureChain(provider);
  const [account] = await client.requestAddresses();
  if (!account) throw new Error("No wallet account selected.");
  const signature = await client.signTypedData({
    account,
    domain: { name: "HyperliquidSignTransaction", version: "1", chainId: HYPERLIQUID_SIGNATURE_CHAIN_ID, verifyingContract: "0x0000000000000000000000000000000000000000" },
    types,
    primaryType,
    message: action,
  });
  return { action, signature };
}

function hyperliquidActionBase() {
  return { hyperliquidChain: "Mainnet", signatureChainId: "0xa4b1", nonce: Date.now() };
}

function initApiWalletSetup() {
  const form = document.querySelector<HTMLFormElement>("[data-api-wallet-form]");
  const submit = form?.querySelector<HTMLButtonElement>("[data-api-wallet-submit]");
  const toggle = form?.querySelector<HTMLButtonElement>("[data-api-wallet-import-toggle]");
  const importSection = document.querySelector<HTMLElement>("[data-api-wallet-import]");
  const privateKey = importSection?.querySelector<HTMLInputElement>('input[name="hyperliquid_private_key"]');
  if (submit && toggle && importSection && privateKey) {
    let walletSource: "generate" | "import" = "generate";
    toggle.addEventListener("click", () => {
      const isHidden = importSection.classList.contains("hidden");
      importSection.classList.toggle("hidden", !isHidden);
      walletSource = isHidden ? "import" : "generate";
      submit.textContent = isHidden ? "Import API Key" : "Generate API Key";
      toggle.textContent = isHidden ? "Generate API key" : "Import existing API key";
      toggle.setAttribute("aria-expanded", isHidden ? "true" : "false");
      privateKey.required = isHidden;
      if (isHidden) {
        privateKey.focus();
      } else {
        privateKey.value = "";
      }
    });
    submit.addEventListener("click", () => {
      void generateAndApproveSigner(walletSource, privateKey, false);
    });
  }
  const regenerate = document.querySelector<HTMLButtonElement>("[data-api-wallet-regenerate]");
  regenerate?.addEventListener("click", () => {
    void generateAndApproveSigner("generate", null, true);
  });
}

async function generateAndApproveSigner(walletSource: "generate" | "import", privateKey: HTMLInputElement | null, force: boolean) {
  const page = document.querySelector<HTMLElement>("[data-account-page]");
  if (!page) return;
  const status = page.querySelector<HTMLElement>("[data-api-wallet-status]");
  const setStatus = (text: string) => { if (status) status.textContent = text; };
  const existingAddress = page.dataset.apiWalletAddress;
  try {
    if (force || !existingAddress) {
      setStatus(force ? "Re-generating trading signer..." : "Generating trading signer...");
      const privateKeyValue = walletSource === "import" ? privateKey?.value ?? "" : "";
      const response = await fetch("/account/api-wallet", {
        method: "POST",
        headers: { "content-type": "application/json", "X-CSRF-Token": csrfToken() ?? "" },
        body: JSON.stringify({ walletSource, hyperliquidPrivateKey: privateKeyValue }),
      });
      const result = (await response.json().catch(() => null)) as { status?: string; api_wallet_address?: string; error?: string } | null;
      if (!response.ok || !result?.api_wallet_address) {
        throw new Error(result?.error ?? "Could not generate trading signer.");
      }
      page.dataset.apiWalletAddress = result.api_wallet_address;
    }
    await approveTradingSigner(page);
  } catch (error) {
    setStatus(error instanceof Error ? error.message : "Trading signer setup failed.");
  }
}

async function approveTradingSigner(page: HTMLElement) {
  const approvalStatus = page.querySelector<HTMLElement>("[data-api-wallet-status]");
  const address = page.dataset.apiWalletAddress;
  if (!approvalStatus || !address) throw new Error("Set up a trading signer first.");
  approvalStatus.textContent = "Awaiting wallet signature...";
  const action = { type: "approveAgent", ...hyperliquidActionBase(), agentAddress: address, agentName: "Vibetrading" };
  const signed = await signWalletAction("HyperliquidTransaction:ApproveAgent", {
    "HyperliquidTransaction:ApproveAgent": [{ name: "hyperliquidChain", type: "string" }, { name: "agentAddress", type: "address" }, { name: "agentName", type: "string" }, { name: "nonce", type: "uint64" }],
  }, action);
  const response = await fetch("/account/approve-api-wallet", { method: "POST", headers: { "content-type": "application/json", "X-CSRF-Token": csrfToken() ?? "" }, body: JSON.stringify(signed) });
  if (!response.ok) {
    const body = await response.json().catch(() => null) as { response?: string; message?: string; status?: string } | null;
    const detail = body?.response ?? body?.message ?? body?.status ?? response.statusText;
    throw new Error(`Hyperliquid did not approve the trading signer: ${detail}`);
  }
  window.location.reload();
}

function initAccountPage() {
  const page = document.querySelector<HTMLElement>("[data-account-page]");
  if (!page) return;
  const slider = page.querySelector<HTMLInputElement>("[data-builder-fee]");
  const input = page.querySelector<HTMLInputElement>("[data-builder-fee-input]");
  const status = page.querySelector<HTMLElement>("[data-builder-fee-status]");
  if (!slider || !input) return;
  const minBps = Number(slider.min);
  const maxBps = Number(slider.max);
  const clampBps = (value: number) => Math.max(minBps, Math.min(maxBps, Math.round(value)));
  const bpsToPercent = (bps: number) => (bps / 100).toFixed(2);
  const syncFromSlider = () => { input.value = bpsToPercent(Number(slider.value)); };
  const syncFromInput = () => {
    const percent = Number(input.value);
    if (!Number.isFinite(percent)) return;
    slider.value = String(clampBps(Math.round(percent * 100)));
  };
  slider.addEventListener("input", syncFromSlider);
  input.addEventListener("input", syncFromInput);
  input.addEventListener("blur", () => { input.value = bpsToPercent(Number(slider.value)); });
  page.querySelector("[data-approve-builder-fee]")?.addEventListener("click", () => {
    if (!status) return;
    void (async () => {
      status.textContent = "Awaiting wallet signature...";
      const feeBps = clampBps(Math.round(Number(input.value) * 100));
      slider.value = String(feeBps);
      input.value = bpsToPercent(feeBps);
      const maxFeeRate = `${(feeBps / 100).toFixed(2)}%`;
      const action = { type: "approveBuilderFee", ...hyperliquidActionBase(), maxFeeRate, builder: page.dataset.builderRecipient ?? "" };
      const signed = await signWalletAction("HyperliquidTransaction:ApproveBuilderFee", {
        "HyperliquidTransaction:ApproveBuilderFee": [{ name: "hyperliquidChain", type: "string" }, { name: "maxFeeRate", type: "string" }, { name: "builder", type: "address" }, { name: "nonce", type: "uint64" }],
      }, action);
      const response = await fetch("/account/approve-builder-fee", { method: "POST", headers: { "content-type": "application/json", "X-CSRF-Token": csrfToken() ?? "" }, body: JSON.stringify({ ...signed, feeBps }) });
      if (!response.ok) throw new Error("Hyperliquid did not approve the fee.");
      status.textContent = "Approved on Hyperliquid.";
      const result = await response.json().catch(() => null) as { redirect?: string } | null;
      window.location.assign(result?.redirect ?? "/agents/new");
    })().catch((error: unknown) => { if (status) status.textContent = error instanceof Error ? error.message : "Approval failed."; });
  });
}

function canonicalTransferAmount(value: string): string | null {
  if (!/^\d+(?:\.\d+)?$/.test(value) || value.includes("e") || value.includes("E")) {
    return null;
  }
  const [integer, fraction = ""] = value.split(".");
  if (fraction.length > 8) return null;
  const normalizedInteger = integer.replace(/^0+(?=\d)/, "");
  const normalizedFraction = fraction.replace(/0+$/, "");
  if (BigInt(normalizedInteger) === 0n && normalizedFraction.length === 0) return null;
  return normalizedFraction.length > 0
    ? `${normalizedInteger}.${normalizedFraction}`
    : normalizedInteger;
}

function initAccountTransfers() {
  const form = document.querySelector<HTMLFormElement>("[data-transfer-form]");
  const from = form?.querySelector<HTMLSelectElement>("[data-transfer-from]");
  const to = form?.querySelector<HTMLSelectElement>("[data-transfer-to]");
  const amount = form?.querySelector<HTMLInputElement>("[data-transfer-amount]");
  const submit = form?.querySelector<HTMLButtonElement>("[data-transfer-submit]");
  const status = form?.querySelector<HTMLElement>("[data-transfer-status]");
  const page = document.querySelector<HTMLElement>("[data-account-page]");
  if (!form || !from || !to || !amount || !submit || !status || !page) return;

  const updateState = () => {
    const validPair = from.value !== "" && to.value !== "" && from.value !== to.value;
    const transfersEnabled = page.dataset.transfersEnabled === "true";
    submit.disabled = !transfersEnabled || !validPair || canonicalTransferAmount(amount.value) === null;
    if (!transfersEnabled && status.textContent === "") {
      status.textContent = "Transfers require a verified Unified Account and at least two accounts.";
    }
  };
  const repairPair = (changed: HTMLSelectElement, other: HTMLSelectElement) => {
    if (changed.value !== other.value) return;
    const different = Array.from(other.options).find((option) => option.value !== changed.value);
    if (different) other.value = different.value;
  };
  from.addEventListener("change", () => { repairPair(from, to); updateState(); });
  to.addEventListener("change", () => { repairPair(to, from); updateState(); });
  amount.addEventListener("input", updateState);
  updateState();

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const canonicalAmount = canonicalTransferAmount(amount.value);
    if (!canonicalAmount || from.value === to.value) {
      updateState();
      status.textContent = "Choose two different accounts and enter a positive USDC amount with up to 8 decimals.";
      return;
    }
    const mainAddress = page.dataset.mainAddress ?? "";
    const action = {
      type: "sendAsset",
      hyperliquidChain: "Mainnet",
      signatureChainId: "0xa4b1",
      destination: to.value,
      sourceDex: "spot",
      destinationDex: "spot",
      // Canonical USDC tokenId from Hyperliquid mainnet spotMeta.
      token: "USDC:0x6d1e7cde53ba9467b783cb7c530ce054",
      amount: canonicalAmount,
      fromSubAccount: from.value === mainAddress ? "" : from.value,
      nonce: Date.now(),
    };
    submit.disabled = true;
    status.textContent = "Awaiting wallet signature...";
    void signWalletAction("HyperliquidTransaction:SendAsset", {
      "HyperliquidTransaction:SendAsset": [
        { name: "hyperliquidChain", type: "string" },
        { name: "destination", type: "string" },
        { name: "sourceDex", type: "string" },
        { name: "destinationDex", type: "string" },
        { name: "token", type: "string" },
        { name: "amount", type: "string" },
        { name: "fromSubAccount", type: "string" },
        { name: "nonce", type: "uint64" },
      ],
    }, action)
      .then(async (signed) => {
        status.textContent = "Submitting transfer...";
        const response = await fetch("/account/transfers", {
          method: "POST",
          headers: { "content-type": "application/json", "X-CSRF-Token": csrfToken() ?? "" },
          body: JSON.stringify(signed),
        });
        const body = await response.json().catch(() => null) as { error?: string; response?: string; message?: string; status?: string } | null;
        if (!response.ok) throw new Error(body?.error ?? body?.response ?? body?.message ?? body?.status ?? "Transfer failed.");
        window.location.reload();
      })
      .catch((error: unknown) => {
        status.textContent = error instanceof Error ? error.message : "Transfer failed.";
        updateState();
      });
  });
}

function initAgentCreation() {
  const page = document.querySelector<HTMLElement>("[data-agent-creation]");
  if (!page) return;
  const button = page.querySelector<HTMLButtonElement>("[data-create-new-agent-subaccount]");
  const status = page.querySelector<HTMLElement>("[data-new-agent-subaccount-status]");
  const refresh = page.querySelector<HTMLElement>("[data-new-agent-subaccount-refresh]");
  button?.addEventListener("click", () => {
    void (async () => {
      const nameInput = page.querySelector<HTMLInputElement>("#display_name");
      const displayName = nameInput?.value.trim() ?? "";
      if (!displayName) {
        nameInput?.focus();
        throw new Error("Enter an agent name first.");
      }
      if (!status || !button) return;
      button.disabled = true;
      status.textContent = "Creating Sub-Account with the server trading signer...";
      const response = await fetch("/account/subaccounts", {
        method: "POST",
        headers: { "content-type": "application/json", "X-CSRF-Token": csrfToken() ?? "" },
        body: JSON.stringify({ displayName }),
      });
      const result = (await response.json().catch(() => null)) as { name?: string; created?: boolean; error?: string } | null;
      if (!response.ok || !result?.name) throw new Error(result?.error ?? "Sub-Account creation failed.");
      status.textContent = result.created ? "Sub-Account created and selected below." : "Existing Sub-Account selected below.";
      refresh?.dispatchEvent(new CustomEvent("newAgentSubaccountCreated", { detail: { name: result.name } }));
    })().catch((error: unknown) => {
      if (status) status.textContent = error instanceof Error ? error.message : "Sub-Account creation failed.";
    }).finally(() => {
      if (button) button.disabled = false;
    });
  });
}

function blockieDataUri(address: string): string {
  let seed = 0;
  for (let index = 0; index < address.length; index += 1) {
    seed = ((seed << 5) - seed + address.charCodeAt(index)) >>> 0;
  }

  const random = () => {
    seed = (seed * 9301 + 49297) % 233280;
    return seed / 233280;
  };
  const color = () => `hsl(${Math.floor(random() * 360)} ${Math.floor(random() * 60 + 40)}% ${Math.floor((random() + random() + random() + random()) * 25)}%)`;
  const foreground = color();
  const background = color();
  const spot = color();
  const squares: string[] = [];

  for (let row = 0; row < 8; row += 1) {
    const values: number[] = [];
    for (let column = 0; column < 4; column += 1) {
      values.push(Math.floor(random() * 2.3));
    }
    values.push(...values.slice(0, 4).reverse());
    values.forEach((value, column) => {
      if (value !== 0) {
        squares.push(`<rect x="${column}" y="${row}" width="1" height="1" fill="${value === 1 ? foreground : spot}"/>`);
      }
    });
  }

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8" fill="${background}"/>${squares.join("")}</svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

function initAccountNavbar() {
  const link = document.querySelector<HTMLElement>("[data-account-navbar]");
  const label = link?.querySelector<HTMLElement>("[data-account-navbar-label]");
  const identicons = document.querySelectorAll<HTMLImageElement>("[data-account-identicon]");
  const address = document.querySelector<HTMLElement>("[data-account-page]")?.dataset.accountAddress;
  const setAddress = (value: string) => {
    identicons.forEach((identicon) => {
      identicon.src = blockieDataUri(value);
      identicon.classList.remove("hidden");
    });
    if (label && value.length > 10) label.textContent = `${value.slice(0, 6)}...${value.slice(-4)}`;
  };
  if (address) { setAddress(address); return; }
  void fetch("/account/address").then((response) => response.ok ? response.json() : null).then((data: { wallet_address?: string } | null) => {
    if (data?.wallet_address) setAddress(data.wallet_address);
  });
}

function init() {
  renderTimeago();
  renderLocalDateTimes();
  initMemoryTimelineDragScroll();
  initApiWalletSetup();
  initAgentRailTransition();
  initAgentRailEntryNavigation();
  initAgentRailNavigation();
  syncAgentSelector();
  initAgentSelectorDismissal();
  initCopyButtons();
  initDetailDeleteModal();
  initAccountPage();
  initAccountTransfers();
  initAgentCreation();
  initAccountNavbar();
  seedSelectedMemoryTimelineItems();
  restoreSelectedMemoryTimelineItem();
  startRunningDurationTicker();

  document.querySelectorAll<HTMLElement>(".number-roll").forEach(seedNumberRoll);

  document.addEventListener("htmx:sseMessage", (e: Event) => {
    const detail = (e as CustomEvent).detail;
    console.debug("htmx SSE message", detail?.type ?? detail?.event?.type ?? detail?.elt);
    if (detail?.type === "balance" || detail?.type === "positions") {
      setTimeout(() => {
        document.querySelectorAll<HTMLElement>(".number-roll").forEach(animateNumberRoll);
      }, 50);
    }
    const target = detail?.elt;
    if (target instanceof Element && target.matches('[sse-swap="memories-timeline"]')) {
      restoreSelectedMemoryTimelineItem(target.closest("[data-agent-memories]") ?? document);
    }
  });

  document.addEventListener("htmx:afterSwap", (e: Event) => {
    const detail = (e as CustomEvent).detail;
    const target = detail?.target;
    if (!(target instanceof Element)) {
      return;
    }

    if (target.id === "agent-selector-items") {
      syncAgentSelector();
    }

    if (
      target.matches('[sse-swap="memories-timeline"]') ||
      target.querySelector('[sse-swap="memories-timeline"]')
    ) {
      seedSelectedMemoryTimelineItems();
      restoreSelectedMemoryTimelineItem();
    }
  });

  window.addEventListener("pageshow", (event) => {
    const rail = document.querySelector(".agent-rail");
    rail?.classList.remove("agent-rail-exit");
    if (event.persisted) {
      rail?.classList.add("agent-rail-enter");
    }
  });

  document.addEventListener("htmx:sseOpen", (e: Event) => {
    const detail = (e as CustomEvent).detail;
    console.debug("htmx SSE open", detail?.elt);
  });

  document.addEventListener("htmx:sseError", (e: Event) => {
    const detail = (e as CustomEvent).detail;
    console.debug("htmx SSE error", detail?.error ?? detail?.source ?? detail);
  });

  document.addEventListener("htmx:afterRequest", (e: Event) => {
    const detail = (e as CustomEvent).detail;
    if (!detail?.successful) {
      return;
    }

    const elt = detail.elt;
    if (elt instanceof HTMLElement && elt.matches("[data-memory-timeline-item]")) {
      setActiveMemoryTimelineItem(elt);
    }
  });

  if (typeof MutationObserver === "undefined") {
    return;
  }

  const observer = new MutationObserver((mutations) => {
    for (const mutation of mutations) {
      for (const node of mutation.addedNodes) {
        if (!(node instanceof Element)) {
          continue;
        }
        if (node.matches("time.timeago")) {
          render([node as HTMLElement]);
          (node as HTMLElement).classList.add("timeago-ready");
        }
        if (node.matches("time.local-datetime")) {
          renderLocalDateTimeNode(node);
        }
        renderTimeago(node);
        renderLocalDateTimes(node);
        seedSelectedMemoryTimelineItems(node);
        restoreSelectedMemoryTimelineItem(node);
        if (node instanceof HTMLElement) {
          if (node.matches(".memory-timeline-scroll")) {
            initMemoryTimelineDragScroll();
          }
          if (node.querySelector(".memory-timeline-scroll")) {
            initMemoryTimelineDragScroll();
          }
        }
      }
    }
  });

  observer.observe(document.documentElement, {
    childList: true,
    subtree: true,
  });
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", init);
} else {
  init();
}
