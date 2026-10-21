import type { ReactNode } from "react";

const actionClassName = "inline-flex cursor-pointer items-center justify-center rounded-full border border-violet-500 px-5 py-2 text-sm font-medium text-violet-300 transition-colors hover:bg-violet-950 focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-violet-400";

function RouteStatusPage({ status, title, description, children }: {
  status: string;
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <main className="grid min-h-dvh place-items-center p-6">
      <section aria-labelledby="page-status-heading" className="w-full max-w-lg rounded-2xl border border-zinc-800 bg-[#0d0d10] p-8 text-center sm:p-10">
        <a href="/" className="inline-flex cursor-pointer items-center gap-3 rounded-md text-lg font-semibold tracking-wide focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-violet-400">
          <img src="/static/logo.png" alt="" className="h-12 w-auto rounded-md object-contain" />
          <span>HyperVibes</span>
        </a>
        <p className="mt-10 font-mono text-sm font-medium uppercase tracking-widest text-violet-400">{status}</p>
        <h1 id="page-status-heading" className="mt-3 text-2xl font-semibold tracking-tight text-zinc-100 sm:text-3xl">{title}</h1>
        <p className="mt-3 text-sm leading-6 text-zinc-400">{description}</p>
        <div className="mt-8 flex flex-wrap justify-center gap-3">{children}</div>
      </section>
    </main>
  );
}

export function NotFoundPage() {
  return (
    <RouteStatusPage status="404 · Not found" title="Page not found" description="The page you’re looking for doesn’t exist or may have been moved.">
      <a href="/" className={actionClassName}>Go home</a>
    </RouteStatusPage>
  );
}

export function RouteErrorPage() {
  return (
    <RouteStatusPage status="Unexpected error" title="Something went wrong" description="We couldn’t load this page. Try again, or head back home to continue.">
      <button type="button" onClick={() => window.location.reload()} className={actionClassName}>Try again</button>
      {/* Full navigation also recovers when the current router or root layout has failed. */}
      <a href="/" className="inline-flex cursor-pointer items-center justify-center rounded-full border border-zinc-700 px-5 py-2 text-sm font-medium text-zinc-300 transition-colors hover:bg-zinc-800 focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-violet-400">Go home</a>
    </RouteStatusPage>
  );
}
