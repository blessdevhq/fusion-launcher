# Canonical primitives — Fusion Launcher

Only listed components are canonical starting points. This document does not authorize a new component catalogue; it makes existing patterns discoverable to agents.

| Primitive | Purpose | Allowed variants / states | Accessibility and interaction notes | Source path |
|---|---|---|---|---|
| Desktop window frame | Native-feeling title bar and window controls | default, hover, keyboard focus, close warning | Focus remains visible; close retains destructive-state affordance | `src/components/shell/DesktopWindowFrame.tsx` |
| App shell and sidebar | Primary navigation and desktop work area | default, active nav, keyboard/gamepad focus | Uses `data-focus-id` / `rh-focusable`; do not create competing navigation semantics | `src/components/shell/AppShell.tsx`, `Sidebar.tsx` |
| Controller hint bar | Discoverable controller/keyboard actions | contextual action set | Must stay clear of scrollable screen content | `src/components/shell/ControllerHintBar.tsx` |
| Home cockpit panels | Immediate library status and next action | populated, empty, skeleton/loading | Preserve clear hero and task priority | `src/components/dashboard/HomeScreen.tsx`, `CockpitPanels.tsx` |
| Game poster | Artwork with metadata/fallback behavior | image, fallback, selected | Artwork cannot be the only carrier of critical state | `src/components/shell/GamePoster.tsx` |
| Game details modal | Details, setup progression and install actions | default, checklist, download, imported, error | Focusable actions and scroll behavior stay usable | `src/components/GameDetailsModal.tsx` |
| Settings modal | Settings sections and source/emulator controls | tabbed sections, save/dirty state, localized UI | Preserve keyboard access and nested scroll container | `src/components/SettingsModal.tsx` |
| Downloads screen | Download progress, queue and errors | active, ready, completed, error | Progress and error must remain readable without color alone | `src/components/dashboard/DownloadsScreen.tsx` |

## Rules for agents

1. Search for a matching primitive, test id or visual pattern before creating a component.
2. Reuse `--rh-*` CSS roles and `fusion-*` Tailwind names; do not introduce raw visual literals in product components unless an approved local exception is documented.
3. Propose a new primitive only after demonstrating repeated use or a clear mismatch with the existing task-specific intent.
4. Add representative states and a source-path entry together with a new canonical primitive.
