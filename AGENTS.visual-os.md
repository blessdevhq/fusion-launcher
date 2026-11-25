# Visual OS instructions — merge into AGENTS.md deliberately

For UI work, first read the relevant files under `design/` and `.agents/visual-os/`.

1. Search for existing tokens, primitives, stories and route patterns before creating new UI.
2. Implement from the screen contract. Use semantic tokens and canonical primitives; do not introduce raw visual literals or incidental global-style changes.
3. Treat Figma/screenshots as design intent. Translate them into the repository's real token, component and accessibility system.
4. Inspect rendered desktop, mobile and required state output. Run focused UI/a11y/visual checks when present.
5. Report intended visual diffs. Never autonomously approve screenshot baselines, new shared tokens or new canonical primitives.
