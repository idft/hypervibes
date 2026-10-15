import { createServerFn } from "@tanstack/react-start";
import { getRequestHeaders } from "@tanstack/react-start/server";
import { currentUser } from "../auth-user.js";

export const getCurrentUser = createServerFn({ method: "GET" }).handler(
  ({ context }) => currentUser(context.runtime, getRequestHeaders()),
);
