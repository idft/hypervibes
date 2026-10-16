import { createFileRoute, redirect } from "@tanstack/react-router";
import { LoginPage } from "../components/login-page.js";
import { getCurrentUser } from "../web/auth.js";
import { getLoginDestination } from "../web/providers.js";

export const Route = createFileRoute("/login")({
  beforeLoad: async () => {
    if (await getCurrentUser()) throw redirect({ to: await getLoginDestination() });
  },
  head: () => ({ meta: [{ title: "Sign in - HyperVibes" }] }),
  component: LoginPage,
});
