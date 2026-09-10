# Screen contract — Home screen with a connected source

## User task and success

- **User / moment:** A Windows player opens an already configured Fusion Launcher to orient themselves and decide what to play, install or manage next.
- **Job to be done:** Understand library readiness and reach the most relevant game/library action without scanning an undifferentiated dashboard.
- **Primary decision or action:** Select the current game/library path or navigate to Library, Downloads or Settings.
- **Failure or confusion to avoid:** Decorative shell elements, artwork or status panels obscure the next action, controller hints, focus target or scroll boundary.

## Information hierarchy

1. Current library/game context and the next meaningful action.
2. Readiness, setup and download status that affect the action.
3. Stable navigation and supporting controller/keyboard hints.

## Visual intent

- **Brand moves applied:** dark cockpit shell, real game artwork as library content, restrained green readiness/focus signals, operational text before decoration.
- **Canonical primitives to reuse:** `AppShell`, `DesktopWindowFrame`, `Sidebar`, `ControllerHintBar`, `HomeScreen`, `CockpitPanels`, `GamePoster`.
- **New exception, if any, and rationale:** none without human approval.

## Required states

- [x] Populated default: connected source and home content are visible.
- [x] Wide desktop: 1280×720, 1365×768, 1440×900 and 1920×1080 remain usable.
- [x] Loading / skeleton: startup content is not mistaken for an empty library.
- [ ] Empty: covered by the separate first-run onboarding contract when no source is connected.
- [x] Focus / selected: keyboard and gamepad navigation visibly identify the active target.
- [x] Reduced motion: new home motion respects `prefers-reduced-motion`.

## Acceptance evidence

- **Target route:** web shell root with connected demo source.
- **Desktop viewports:** 1280×720, 1365×768, 1440×900 and 1920×1080.
- **Mobile:** out of scope; Fusion Launcher is a Windows desktop product. Do not claim mobile support from this contract.
- **Browser / visual test:** `npm run mvp:visual`, including home screenshots, overflow checks, focus targets and controller-hint clearance.
- **Accessibility / keyboard check:** preserve visible `rh-focusable` / `data-focus-active` behavior and ensure no horizontal overflow.
- **Human approval required for:** new home hierarchy, shared roles, shell primitives or any approved screenshot baseline in the future.
