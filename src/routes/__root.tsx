import { createRootRoute, HeadContent, Outlet, Scripts } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { Navbar } from "../components/navbar.js";
import stylesheet from "../web/app.css?url";

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1" },
      { title: "HyperVibes" },
    ],
    links: [
      { rel: "stylesheet", href: stylesheet },
      { rel: "icon", href: "/static/favicon.ico" },
      { rel: "manifest", href: "/static/site.webmanifest" },
    ],
  }),
  component: Layout,
  shellComponent: Document,
  notFoundComponent: () => <p className="text-zinc-400">Page not found.</p>,
  errorComponent: () => <p role="alert" className="text-red-300">Unable to load this page. Please try again.</p>,
});

function Layout() {
  return (
    <>
      <header className="fixed inset-x-0 top-0 z-40 h-14 border-b border-zinc-800 bg-[#17191c]">
        <Navbar />
      </header>
      <div className="app-shell fixed inset-x-0 bottom-0 top-14 overflow-y-auto">
        <main className="app-content min-w-0 px-4 py-6 sm:px-6 lg:px-8"><Outlet /></main>
      </div>
    </>
  );
}

function Document({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <head><HeadContent /></head>
      <body className="bg-[#0a0a0b] text-zinc-100 antialiased">
        {children}
        <Scripts />
      </body>
    </html>
  );
}
