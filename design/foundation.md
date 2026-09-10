# Visual foundation — Fusion Launcher

## Product-independent grammar

- **Density:** A calm, information-dense Windows desktop workspace. Preserve breathing room at decisions; do not turn every data group into a card.
- **Hierarchy:** The current game, next safe action, and operational status outrank decoration. Game artwork supports recognition but must not obscure controls or status.
- **Typography philosophy:** Inter and the existing system fallbacks carry operational UI. Use weight and opacity before adding display typefaces.
- **Surface and contrast philosophy:** Near-black layered surfaces, restrained borders, and legible white/muted text. The green accent signals ready/selected/progress, not every interactive element.
- **Shape / rhythm:** Rounded but not toy-like surfaces; the established 8/12/16/20 px radius scale is intentional.
- **Motion temperament:** Brief transitions clarify state. Respect `prefers-reduced-motion`; no decorative looping or heavy compositing in routine workflows.

## Reference-calibrated library mode

The user-provided game-library references establish an art direction, not a screen
template. Their transferable quality is a cinematic game-selection moment with a
quiet operating layer around it.

- **One featured game, then a rail:** In library-facing home states, give one selected
  game a clearly dominant hero scene. A compact horizontal run of neighbouring covers
  or a queue supplies breadth without competing with the selected title.
- **Artwork leads locally, chrome stays neutral:** A hero may use its own authorised
  artwork, palette and contrast scrim for atmosphere. Navigation, setup, diagnostics
  and global controls retain the neutral Fusion surface language and green semantic
  signals; a game's colour must never redefine product state.
- **Controls form a readable foreground plane:** Title, short factual context and one
  launch/setup action sit over a deliberate dark scrim or a quiet lower edge of the
  hero. Do not place operational text directly on an uncontrolled image region.
- **Curated rhythm over uniform grids:** Use a selected cover, a horizontal title rail
  and a small number of asymmetrical artwork blocks when browsing is the task. Use
  lists, rows and panels when comparing status, sources, storage or errors.
- **Contained immersion:** The artwork can feel expansive inside the application frame,
  while persistent navigation and system controls remain stable, narrow and visually
  subordinate. The desktop shell must still make keyboard and controller focus obvious.

## Mode boundary

- **Library and discovery:** Cinematic hierarchy is appropriate: selected game first,
  related titles second, operational metadata third.
- **Installation, downloads, settings and recovery:** Operational hierarchy wins:
  explicit progress, provenance, errors and next safe actions must sit on stable opaque
  surfaces. Artwork is optional supporting context, never a background for decisions.

## Canonical implementation layer

- `src/app/globals.css` owns CSS variables such as `--rh-bg`, `--rh-surface`, `--rh-accent`, radius and shadow roles.
- `tailwind.config.js` exposes the aligned `fusion.*` Tailwind names.
- `design/tokens.json` is an agent-readable role map, not a second generator or a replacement for the code sources above.

## Non-negotiables

- Reuse `--rh-*` roles and `fusion-*` utilities before adding visual literals.
- Keyboard and gamepad focus remains visible and distinct from selection.
- Repeated visual values become shared roles only after repeated production use.
- New shared roles, canonical primitives and visual baselines require human approval.
- The launcher remains usable as a Windows desktop application at the supported wide viewports.

## Avoid by default

- Generic neon/glass dashboard styling, loud gradients, and glow on every control.
- Steam-like storefront chrome or retro pixel decoration that competes with real game artwork.
- A card grid when a list, status row, shell panel or direct action better serves the task.
- A replica of a console, streaming-service or game-store layout: no copied navigation
  silhouettes, control placement, artwork treatment, copy or branded visual cues.
- Full-bleed imagery without an intentional readable plane for controls, focus and
  metadata; atmosphere is never a reason to weaken a launch or recovery decision.

## Anti-slop rules

The user-provided anti-references identify a failure mode to actively reject: a
generic dark SaaS/AI dashboard with decorative light, nested cards and invented
importance. Apply these checks during implementation and review.

- **No decorative glow system:** Do not use coloured outer glow, bloom, aurora blobs or
  a gradient background as the default identity of the shell. A game hero may have a
  local contrast scrim; a transient state may use one restrained semantic highlight.
  Neither is permission to make every panel luminous.
- **No card-inside-card inflation:** Each surface must represent a meaningful task,
  content group or state. Do not wrap rows, metrics, actions and their parent section
  in separate rounded bordered containers just to make the screen look “designed”.
- **No invented KPI theatre:** Do not add growth metrics, activity graphs, generic
  completion percentages, badges or recommendation modules unless they are backed by
  real launcher data and directly help a library, setup or download decision.
- **No synthetic AI hero:** Do not use vague promises, floating 3D objects, abstract
  purple/cyan imagery or ambient particles as a primary hero. A prominent visual must
  be authorised game content, an actionable system state, or an explicit empty/error
  state with a next action.
- **No uniform visual emphasis:** Primary action, selected game, error/recovery state
  and secondary metadata need visibly different weight. If every tile has the same
  radius, border, saturation, shadow and icon treatment, reduce rather than decorate.
- **No ornamental controls:** Every icon, chip, toggle, badge and toolbar item needs a
  concrete launcher action or live state. Remove controls whose role cannot be stated
  in one sentence.

### Narrowly permitted

- A restrained gradient or soft image treatment is allowed only inside an authorised
  game-art hero, where the text/action contrast is tested.
- Green may indicate live ready, selected, progress or success states; it is not a
  generic accent for every card, icon and divider.
- Rounded surfaces and subtle shadows are allowed when they establish a real desktop
  layer, modal or selectable game item—not as repeated ornament on every row.

## Accessibility baseline

- Preserve visible focus for keyboard and controller navigation.
- Keep foreground/background contrast sufficient for operational text and state.
- Keep `prefers-reduced-motion` behavior for new animated UI.
- Test wide desktop layouts for overflow, nested scrolling and reachable critical actions.
