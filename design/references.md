# Visual references — Fusion Launcher

External references are for intent, never for copying protected assets, text, screenshots, CSS, logos or distinctive trade dress. Record provenance before adding a new external reference.

| Source URL | What we learn | Must not copy | License / access notes |
|---|---|---|---|
| Internal: `src/app/globals.css` | Existing surface, focus, shell, status and motion language | Raw CSS snippets without understanding the role | Canonical code source |
| Internal: `src/components/shell/AppShell.tsx` | Desktop shell and sidebar composition | Layout assumptions in unrelated screens | Canonical shell primitive |
| Internal: `src/components/dashboard/HomeScreen.tsx` | Home hierarchy and game-library context | Content-specific markup in a different workflow | Current home-screen implementation |
| Internal: `scripts/mvp-visual.mjs` | Rendered flows, visual evidence and wide desktop constraints | Test-only selectors as visual architecture | Existing release visual smoke gate |

## Internal references

| Route or component | Why it is canonical | States to preserve |
|---|---|---|
| `AppShell` / `DesktopWindowFrame` | The desktop shell, sidebar and title-bar grammar | keyboard/controller focus, active nav, native window controls |
| `HomeScreen` / `CockpitPanels` | Home task hierarchy and hero treatment | populated library, skeleton/loading, overflow-free wide viewports |
| `LibraryScreen` / `GameDetailsModal` | Game discovery, details and setup progression | search, selected game, checklist hidden/shown, imported state |
| `SettingsModal` | Settings information architecture and nested scrolling | sources, diagnostics, updates, emulators and Russian UI state |
