import handler, { createServerEntry } from "@tanstack/react-start/server-entry";

// The Node host supplies the runtime in options.context. No startup side effects.
export default createServerEntry({
  fetch(request, options) {
    return handler.fetch(request, options);
  },
});
