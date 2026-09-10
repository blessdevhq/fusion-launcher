# Fusion Launcher UI implementation contract

Before editing UI:

1. Read `design/foundation.md`, `design/brand.md`, `design/components.md`, `design/tokens.json` and the relevant screen contract.
2. Search for existing `--rh-*` roles, `fusion-*` utilities, canonical primitives, `data-testid` selectors and `data-focus-id` patterns before creating UI.
3. State a short visual intent note: task hierarchy, brand moves, primitives to reuse and required states.

While editing:

- `src/app/globals.css` and `tailwind.config.js` are canonical token sources. `design/tokens.json` is a role map only.
- Use `--rh-*` roles and `fusion-*` names before new raw colors, radii, shadows or spacing values.
- Preserve the distinction between active selection and keyboard/gamepad focus.
- Do not add a generic dashboard card grid, new icon set, copied external visuals or incidental global-style changes.
- Do not change shared roles, canonical shell primitives or release visual expectations as an incidental part of a feature.

Before completion:

- Inspect the supported desktop viewport(s) and every applicable critical state; do not invent a mobile requirement for this Windows-only product.
- Check visible focus, contrast, overflow, nested scroll, loading/error behavior and controller hints where applicable.
- Run `npm run mvp:visual` for changes affecting covered launcher flows. Run the focused existing test/checks required by the touched code.
- Report intended visual diffs. Never approve a future screenshot baseline, new shared role or new canonical primitive yourself.
