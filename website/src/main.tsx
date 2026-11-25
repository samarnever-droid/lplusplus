import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import App from "./App";
import { LppClerkProvider } from "./lib/clerk";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <LppClerkProvider>
      <App />
    </LppClerkProvider>
  </StrictMode>
);
