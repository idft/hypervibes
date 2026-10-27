import { createRoot } from "react-dom/client";
import { RouterProvider } from "@tanstack/react-router";
import { createDashboardScreenshotRouter } from "./dashboard-screenshot.js";
import "../web/app.css";

declare global {
  interface Window { __screenshotBtcPrice?: string }
}

const root = document.getElementById("screenshot-root");
if (!root) throw new Error("Screenshot root is missing.");
const price = window.__screenshotBtcPrice;
if (!price) throw new Error("The capture script must supply a fresh BTC quote.");
createRoot(root).render(<RouterProvider router={createDashboardScreenshotRouter(price)} />);
