import { createServerFn } from "@tanstack/react-start";

// Public preview status only. Future private operations must enforce auth here.
export const getRuntimeStatus = createServerFn({ method: "GET" }).handler(
  ({ context }) => context.runtime.status(),
);
