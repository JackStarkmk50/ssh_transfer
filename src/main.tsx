import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";
import "./additional.css";
import "./polish.css";
import App from "./ShelfApp";

createRoot(document.getElementById("root")!).render(
  <StrictMode><App /></StrictMode>
);
