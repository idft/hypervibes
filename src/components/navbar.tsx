import { Link } from "@tanstack/react-router";

export function Navbar() {
  return (
    <div className="relative flex h-full items-center px-4">
      <span className="text-sm text-zinc-400">Select an agent</span>
      <Link to="/agents" className="absolute left-1/2 flex h-full -translate-x-1/2 cursor-pointer items-center gap-2 px-4 text-sm font-semibold tracking-wide text-zinc-100 hover:bg-zinc-800">
        <img src="/static/android-chrome-512x512.png" alt="" className="h-9 w-9 rounded-md object-cover" />
        <span>HyperVibes</span>
      </Link>
      <div className="ml-auto flex items-center gap-3">
        <span className="rounded-full border border-zinc-700 px-2 py-0.5 text-xs text-zinc-400">Preview</span>
        <a href="https://github.com/idft/hypervibes" target="_blank" rel="noopener noreferrer" aria-label="HyperVibes on GitHub" className="inline-flex h-9 w-9 cursor-pointer items-center justify-center rounded-lg text-zinc-400 hover:bg-zinc-800 hover:text-white">
          <span className="github-icon h-5 w-5" aria-hidden="true" />
        </a>
      </div>
    </div>
  );
}
