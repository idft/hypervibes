import htmx from "htmx.org";
import "htmx-ext-sse";

(window as Window & { htmx?: typeof htmx }).htmx = htmx;

export { htmx };
