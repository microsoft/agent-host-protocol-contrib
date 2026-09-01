import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import "./index.css"
import App from "./App"

const colorScheme = window.matchMedia("(prefers-color-scheme: dark)")
const syncSystemTheme = () => document.documentElement.classList.toggle("dark", colorScheme.matches)
syncSystemTheme()
colorScheme.addEventListener("change", syncSystemTheme)

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
