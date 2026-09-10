# Fusion Launcher visual review checklist

## Product and hierarchy

- [ ] The current game/library context and next action are clear in the first viewport.
- [ ] Status/setup information supports the decision instead of becoming dashboard decoration.
- [ ] Controller hints and stable navigation remain reachable and unobscured.

## Brand and craft

- [ ] The change preserves the calm dark cockpit, restrained green signals and game-art-as-content direction.
- [ ] It avoids generic SaaS, storefront and retro-decoration anti-references.
- [ ] Typography, density, layered surfaces and motion follow `design/foundation.md`.

## System integrity

- [ ] Existing shell, modal, dashboard and artwork primitives were reused where appropriate.
- [ ] `--rh-*` and `fusion-*` roles were reused before raw visual values.
- [ ] A new shared role or canonical primitive has a human-approved rationale and source-path documentation.

## Inclusive desktop behavior

- [ ] Keyboard/gamepad focus remains visible and distinct from selection.
- [ ] Critical text/status contrast remains legible on its actual surface.
- [ ] The applicable wide desktop viewports have been rendered and inspected.
- [ ] Loading, error, selected, disabled and hover states are intentional when applicable.
- [ ] No horizontal overflow, broken nested scroll or obstructed controller hint bar was introduced.

## Evidence and approval

- [ ] Relevant `npm run mvp:visual` evidence was inspected for covered flows.
- [ ] The implementation explains intended visual changes and their product rationale.
- [ ] A human approved any new shared roles, primitives or screenshot baselines.
