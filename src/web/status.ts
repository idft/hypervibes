import { redirect } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { currentUser } from "../auth-user.js";

export const getRuntimeStatus = createServerFn({ method: "GET" }).handler(async ({ context }) => {
  if (!await currentUser(context.runtime, getRequestHeaders())) throw redirect({ to: "/login" });
  return context.runtime.status();
});
