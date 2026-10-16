import { createRootRoute, HeadContent, Outlet, Scripts } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { AppToaster } from "../components/app-toaster.js";
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
  return <><Outlet /><AppToaster /></>;
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
