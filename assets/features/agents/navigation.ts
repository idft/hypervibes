function isAgentDetailPath(pathname: string) {
  return pathname.startsWith("/agents/") && pathname !== "/agents/new";
}

export function syncAgentSelector() {
  const currentAgent = document.querySelector<HTMLElement>("[data-current-agent-key]");
  const agentKey = currentAgent?.dataset.currentAgentKey ?? window.location.pathname.match(/^\/agents\/([^/]+)/)?.[1];
  const selected = currentAgent ?? Array.from(document.querySelectorAll<HTMLElement>("[data-agent-key]")).find((agent) => agent.dataset.agentKey === agentKey);
  const label = document.querySelector<HTMLElement>("[data-agent-selector-label]");
  const status = document.querySelector<HTMLElement>("[data-agent-selector-status]");
  if (!selected || !label || !status) return;
  label.textContent = selected.dataset.currentAgentName ?? selected.dataset.agentName ?? agentKey ?? "";
  status.classList.remove("hidden", "bg-zinc-600", "bg-emerald-400", "bg-red-400");
  status.classList.add((selected.dataset.currentAgentEnabled ?? selected.dataset.agentEnabled) === "true" ? "bg-emerald-400" : "bg-red-400");
}

function syncAgentTabs() {
  document.querySelectorAll<HTMLElement>("[data-agent-tabs]").forEach((nav) => {
    const links = Array.from(nav.querySelectorAll<HTMLAnchorElement>("[data-agent-tab-link]"));
    const active = links.find((link) => new URL(link.href, window.location.origin).pathname === window.location.pathname) ?? links.find((link) => link.ariaCurrent === "page");
    links.forEach((link) => {
      const selected = link === active;
      link.toggleAttribute("aria-current", selected);
      link.classList.toggle("text-white", selected);
      link.classList.toggle("bg-zinc-800", selected);
      link.classList.toggle("text-zinc-400", !selected);
    });
  });
}

export function installAgentNavigation() {
  const entryDestination = window.sessionStorage.getItem("agent-rail-enter-destination");
  window.sessionStorage.removeItem("agent-rail-enter-destination");
  const rail = document.querySelector<HTMLElement>(".agent-rail");
  if (rail && entryDestination === window.location.pathname) rail.classList.add("agent-rail-enter");

  document.addEventListener("click", (event) => {
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    const link = (event.target as Element | null)?.closest<HTMLAnchorElement>("a[href]");
    if (!link || link.hasAttribute("hx-get")) return;
    const destination = new URL(link.href, window.location.href);
    if (destination.origin === window.location.origin && isAgentDetailPath(destination.pathname) && !isAgentDetailPath(window.location.pathname)) {
      window.sessionStorage.setItem("agent-rail-enter-destination", destination.pathname);
    }
  });

  document.addEventListener("click", (event) => {
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    const link = (event.target as Element | null)?.closest<HTMLAnchorElement>("a[href]");
    const currentRail = document.querySelector<HTMLElement>(".agent-rail");
    if (!link || !currentRail || link.hasAttribute("hx-get")) return;
    const destination = new URL(link.href, window.location.href);
    if (destination.origin !== window.location.origin || isAgentDetailPath(destination.pathname)) return;
    event.preventDefault();
    currentRail.classList.add("agent-rail-exit");
    window.setTimeout(() => window.location.assign(destination.href), 180);
  });

  document.addEventListener("click", (event) => {
    const selector = document.querySelector<HTMLDetailsElement>("[data-agent-selector]");
    if (selector?.open && !selector.contains(event.target as Node)) selector.open = false;
  });
  document.addEventListener("htmx:afterSwap", () => {
    syncAgentSelector();
    syncAgentTabs();
  });
  window.addEventListener("popstate", syncAgentTabs);
  syncAgentSelector();
  syncAgentTabs();
}
