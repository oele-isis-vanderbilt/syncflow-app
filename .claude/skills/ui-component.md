---
name: ui-component
description: Use when adding, changing, or reviewing a reusable UI component or page layout in this SvelteKit + Flowbite tauri app, so it follows the design system.
---

# Adding or changing UI

1. **Read the rules.** If not already loaded this session, read
   `docs/design-rules.md` (component table, color and spacing sections).
2. **Search before building.**
   - Does Flowbite Svelte have it? Check the component table in the guide, or
     `node_modules/flowbite-svelte/dist/` for the component folder.
   - Is there already a wrapper in `src/lib/components/ui/`?
   - If either exists, use it. Stop unless it's genuinely insufficient.
3. **Is a wrapper justified?** Only if the same Flowbite combination appears in
   3+ places, or it composes several Flowbite pieces into one unit. Feature-specific components go in `src/lib/features/<feature>/` instead.
4. **Write the wrapper** as `src/lib/components/ui/PascalName.svelte`:
   - Svelte 5 runes; an explicit `Props` interface (no `...rest` passthrough).
   - Variants map to Flowbite `color`/`size` props, not custom classes.
   - Tokens only; add `dark:` pairs for any surface/text classes.
   - Accessible: labels, `aria-label` on icon-only controls, focus rings kept.
   - Use `FormField.svelte` as the reference example.
5. **Verify** from the repo root: `pnpm check`, `pnpm exec prettier --check .`,
   `pnpm check:design`.
6. **Report** what was reused vs. created, and why a new wrapper was needed.
