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

const AGENT_RAIL_EXPANDED_STORAGE_KEY = "agent-rail-expanded";

function agentRailStartsExpanded() {
  return window.localStorage.getItem(AGENT_RAIL_EXPANDED_STORAGE_KEY) !== "false";
}

function setAgentRailExpanded(expanded: boolean) {
  const rail = document.querySelector<HTMLElement>("[data-agent-rail]");
  const content = document.querySelector<HTMLElement>(".app-content.has-agent-rail");
  const toggle = document.querySelector<HTMLButtonElement>("[data-agent-rail-toggle]");
  if (!rail || !content || !toggle) return;

  rail.classList.toggle("agent-rail-expanded", expanded);
  content.classList.toggle("agent-rail-expanded", expanded);
  toggle.setAttribute("aria-expanded", String(expanded));
  toggle.setAttribute("aria-label", expanded ? "Collapse agent navigation" : "Expand agent navigation");
  toggle.title = expanded ? "Collapse agent navigation" : "Expand agent navigation";
  rail.querySelector<SVGElement>("[data-agent-rail-expand-icon]")?.classList.toggle("hidden", expanded);
  rail.querySelector<SVGElement>("[data-agent-rail-collapse-icon]")?.classList.toggle("hidden", !expanded);
}

function syncAgentRail() {
  setAgentRailExpanded(agentRailStartsExpanded());
  document.documentElement.classList.remove("agent-rail-initially-collapsed");
}

function syncAgentTabs() {
  document.querySelectorAll<HTMLElement>("[data-agent-tabs]").forEach((nav) => {
    const links = Array.from(nav.querySelectorAll<HTMLAnchorElement>("[data-agent-tab-link]"));
    const active = links.find((link) => new URL(link.href, window.location.origin).pathname === window.location.pathname) ?? links.find((link) => link.getAttribute("aria-current") === "page");
    links.forEach((link) => {
      const selected = link === active;
      if (selected) link.setAttribute("aria-current", "page");
      else link.removeAttribute("aria-current");
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
  if (rail && entryDestination === window.location.pathname && !agentRailStartsExpanded()) rail.classList.add("agent-rail-enter");

  document.addEventListener("click", (event) => {
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    const link = (event.target as Element | null)?.closest<HTMLAnchorElement>("a[href]");
    if (!link || link.hasAttribute("hx-get")) return;
    const destination = new URL(link.href, window.location.href);
    if (destination.origin === window.location.origin && destination.pathname.startsWith("/agents/") && destination.pathname !== "/agents/new" && !window.location.pathname.startsWith("/agents/")) {
      window.sessionStorage.setItem("agent-rail-enter-destination", destination.pathname);
    }
  });

  document.addEventListener("click", (event) => {
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    const link = (event.target as Element | null)?.closest<HTMLAnchorElement>("a[href]");
    const currentRail = document.querySelector<HTMLElement>(".agent-rail");
    if (!link || !currentRail || link.hasAttribute("hx-get")) return;
    const destination = new URL(link.href, window.location.href);
    if (destination.origin !== window.location.origin || destination.pathname.startsWith("/agents/")) return;
    event.preventDefault();
    currentRail.classList.add("agent-rail-exit");
    window.setTimeout(() => window.location.assign(destination.href), 180);
  });

  document.addEventListener("click", (event) => {
    const toggle = (event.target as Element | null)?.closest<HTMLButtonElement>("[data-agent-rail-toggle]");
    if (!toggle) return;
    const expanded = toggle.getAttribute("aria-expanded") !== "true";
    window.localStorage.setItem(AGENT_RAIL_EXPANDED_STORAGE_KEY, String(expanded));
    setAgentRailExpanded(expanded);
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
  syncAgentRail();
}
