---
paths:
  - "syncflow-publisher/src/**"
---

# Design system

Full guide: `syncflow-publisher/docs/design-rules.md`. If this summary and the guide disagree, the guide wins.

Stack: flowbite-svelte 1.x + Tailwind CSS 4.3 (SvelteKit 3, Svelte 5). This is a desktop app (default window 1000×600), not a website.

1. **Flowbite first.** Before writing UI, check whether `flowbite-svelte` has the component (Button, Input, Select, Toggle, Radio, Checkbox, Modal, Drawer, Card, Table, Tabs, Navbar, Sidebar, Alert, Toast, Badge, Spinner, Progressbar, Tooltip, Pagination…). Use it. Never hand-build what Flowbite provides.
2. **Wrappers second.** Reuse what's in `syncflow-publisher/src/lib/components/ui/`: `SectionCard` for every top-level section (Flowbite 1.x `Card` defaults to `max-w-sm` with no padding), and `ConfirmModal` for destructive actions (except Stop Recording, which must act on the first click). Add a new wrapper only when the same Flowbite component + props combination appears in 3+ places, or to compose several Flowbite pieces into one unit (see the guide's "When to create a wrapper").
3. **Tokens only.**
   - Colors: `primary-*`, `secondary-*`, `gray-*` classes. Status colors (red/green/yellow/blue) only via Flowbite `color` props.
   - No hex/rgb/hsl/oklch values in components, no arbitrary values (`p-[13px]`, `bg-[#123]`, `bg-(--var)`), no other Tailwind palettes (`blue-*`, `slate-*`, and v4.2's `mauve-*`, `olive-*`, `mist-*`, `taupe-*`…), no inline color styles.
   - Tokens are defined only in the `@theme` block of `src/app.css` (Tailwind v4 has no `tailwind.config.js`). Ask before adding one.
   - Links: `text-primary-700 hover:underline`. Never put white text on gold lighter than `primary-600`.
4. **Spacing and layout:** Tailwind's default scale (`1, 2, 3, 4, 6, 8`). The page container (`mx-auto w-full max-w-6xl px-4 py-6`, `space-y-6`) lives once in `+layout.svelte`, so pages don't add their own. Cards `p-4`; groups inside a card `rounded-lg border border-gray-200 p-3`; Flowbite default radius; no gradients, `shadow-xl`, or `hover:scale-*`. `max-w-screen-*` doesn't exist in Tailwind v4. Layouts must fit the default window without horizontal scroll.
5. **Type:** Inter, self-hosted via `@fontsource-variable/inter`. Never load fonts or assets from a CDN (the app runs offline).
6. **Icons:** `flowbite-svelte-icons` only (`…Outline` by default, `…Solid` for filled states).
7. **Light mode only.** No theme toggle exists, so don't add `dark:` classes.
8. **Status and feedback:** session, recording, and streaming states render only through `SessionStatus.svelte` (one color and label per state; the mapping table is in the guide). One `primary` button per view; others `alternative`; destructive `red`. Long-running actions show a `Spinner` and disable their button. Failures (Rust notifications and `invoke` errors) show as `Alert color="red"` with the reason, never only in the console.
9. **Accessibility:** every input has a `Label`; icon-only buttons get `aria-label`; don't rely on color alone for status; never remove focus rings.
10. **Check:** `pnpm check:design` (run from `syncflow-publisher/`) enforces rule 3, plus off-system Flowbite `color` props, hand-written `<svg>`, and gradients. It also runs as a PostToolUse hook on edits. Suppress a line only with `<!-- design-ignore-next-line -->` / `// design-ignore-next-line` and a reason.
