# Design system

Built on **flowbite-svelte 1.x** + **Tailwind CSS 4.3** (SvelteKit 3, Svelte 5). The goal: every screen looks like it came from one designer, whether a person or Claude wrote it.

Paths below are relative to `syncflow-publisher/`.

## Status

All screens and components in `src/` follow these rules, and `pnpm check:design` passes with no suppressions. Keep it that way:

- New code follows these rules.
- When you edit a component, keep it on the shared pieces (`SectionCard`, `ConfirmModal`, `SessionStatus`) instead of re-creating their markup.
- Items marked **(planned)** don't exist yet. Don't assume them; create them only when asked.

## Layers

| Layer               | Where                    | Rule                                                   |
| ------------------- | ------------------------ | ------------------------------------------------------ |
| Tokens              | `src/app.css` (`@theme`) | Only place colors and fonts are defined                |
| Flowbite components | `flowbite-svelte`        | Default choice for all UI                              |
| App wrappers        | `src/lib/components/ui/` | Only when we need project defaults on top of Flowbite  |
| Feature components  | `src/lib/components/`    | Compose the layers above; no custom styling primitives |
| Pages               | `src/routes/**`          | Compose the layers above; no custom styling primitives |

Existing wrappers:

- `ui/SectionCard.svelte`: full-width `Card` (`size="xl" class="p-4"`) with an h2 title, an optional icon, an `actions` snippet, and an optional Show/Hide toggle (`collapsible`, `collapsed`). Use it for every top-level section of a screen.
- `ui/ConfirmModal.svelte`: confirmation `Modal` for destructive actions. It runs an async `onConfirm`, shows a `Spinner` and disables its buttons while it runs, and shows an `Alert` if it throws (throw the reason as a string).

Status component: `src/lib/components/SessionStatus.svelte` is the only place status colors and labels are chosen (see "App-specific states").

## Color

Light mode only for now. `app.css` defines a class-based `dark` variant, but nothing sets `.dark` and no component uses `dark:` classes. Don't add `dark:` classes until a theme toggle exists.

| Role                              | Token                                                | Use for                                 |
| --------------------------------- | ---------------------------------------------------- | --------------------------------------- |
| Primary                           | `primary-50…900`                                     | Main actions, links, focus, active nav  |
| Secondary                         | `secondary-50…900`                                   | Accents, highlights, secondary emphasis |
| Neutral                           | `gray-50…950`                                        | Text, borders, surfaces                 |
| Danger / Success / Warning / Info | Flowbite `color="red" / "green" / "yellow" / "blue"` | Status only, through component props    |

Common pairings:

- Page background: `bg-white`
- Raised surface: `bg-gray-50`
- Border: `border-gray-200`
- Body text: `text-gray-900`
- Muted text: `text-gray-500`
- Link: `text-primary-700 hover:underline`
  (6.35:1 on white, 5.73:1 on `gray-50`. `primary-600` is 4.65:1 on white but only 4.20:1 on `gray-50`, so use it only on white)

To rebrand, change the hex values in `src/app.css` only. Flowbite components pick them up automatically.

Tailwind v4 specifics: there is no `tailwind.config.js`; tokens live only in the `@theme` block of `src/app.css`. `max-w-screen-*` utilities no longer exist (use `max-w-6xl` etc.). The palettes Tailwind 4.2 added (`mauve`, `olive`, `mist`, `taupe`) are non-token palettes like `blue` or `slate`.

Stay on flowbite-svelte 1.x (with the `flowbite` 3.x plugin) until 2.0 is stable. 2.0 moves to Flowbite 4's semantic theme tokens, which will change this color section.

## Typography

- Font: Inter, self-hosted through `@fontsource-variable/inter` (imported in `src/routes/+layout.svelte`, set as `--font-sans` in `@theme`). The app must work offline, so don't load fonts or other assets from a CDN. Monospace uses the system stack.
- Page title: `<Heading tag="h1" class="text-2xl">`
- Section title: `<Heading tag="h2" class="text-xl">`
- Card title: `<Heading tag="h5" class="text-lg">`
- Body: default; small print `text-sm`; captions `text-xs`.
- Use Flowbite's `Heading`, `P`, `A`, `List` typography components rather than raw tags when styling is needed.

## Spacing & layout

This is a desktop app (default window 1000×600), not a website. Favor density over page-style whitespace.

- Use Tailwind's scale only: `1, 2, 3, 4, 6, 8` cover almost everything.
- Page container: `mx-auto w-full max-w-6xl px-4 py-6`, applied once as `<main>` in `src/routes/+layout.svelte`. Pages render their sections directly; don't add another container or `<main>`.
- Section rhythm: `space-y-6` between sections (also set on the layout `<main>`)
- Stack gaps: `gap-2` (tight), `gap-4` (default), `gap-6` (loose)
- Card padding: `p-4`. Flowbite 1.x `Card` defaults to `max-w-sm` and has no padding, so use `SectionCard`, or `Card size="xl" class="p-4"` for a one-off.
- Groups inside a card (a device row, a details block): `rounded-lg border border-gray-200 p-3`, with `bg-gray-50` when they need to stand out. No nested cards or shadows.
- Label/value pairs: a `dl` with `text-sm text-gray-500` terms and `text-gray-900` values.
- Radius: Flowbite defaults (`rounded-lg`); don't mix radii. No `rounded-2xl`, `shadow-xl`, gradients, or `hover:scale-*` effects.
- Layout must work at the default window size without horizontal scroll.

## Components

| Need                               | Use                                                                                           |
| ---------------------------------- | --------------------------------------------------------------------------------------------- |
| Action                             | `Button` (`color="primary"` main, `color="alternative"` secondary, `color="red"` destructive) |
| Text field                         | `Label` + `Input` + `Helper`                                                                  |
| Select / checkbox / radio / toggle | `Select`, `Checkbox`, `Radio`, `Toggle`                                                       |
| Feedback                           | `Alert` (inline), `Toast` (transient)                                                         |
| Container                          | `Card`                                                                                        |
| Overlay                            | `Modal`, `Drawer`, `Dropdown`, `Popover`, `Tooltip`                                           |
| Navigation                         | `Navbar`, `Sidebar`, `Breadcrumb`, `Tabs`, `Pagination`                                       |
| Data                               | `Table` family                                                                                |
| Status                             | `Badge`, `Spinner`, `Progressbar`, `Indicator`                                                |
| Icons                              | `flowbite-svelte-icons` (`…Outline` by default, `…Solid` for filled states)                   |

One primary button per view. Everything else is `alternative` (Refresh, Show/Hide, Back), and destructive actions are `red`. Destructive actions confirm through `ui/ConfirmModal.svelte`: Delete Registration, Reconfigure Devices, and Exit Session. Two actions skip confirmation and act on the first click:

- Stop Recording: the app runs on constrained devices where any extra step delays ending a recording.
- Removing a device from the selection list before starting: it's an in-form edit.

Buttons that contain only an icon need an `aria-label`. A button with an icon and text puts the icon first, `h-4 w-4` with `me-1` (`xs`) or `me-2` (default).

App-specific states (keep these consistent across screens):

- Session, recording, and streaming states always render through `<SessionStatus status=… />`, a `Badge` with a text label plus a pulsing `Indicator` for live states. Don't hand-build status dots or legends.

    | State                       | `status`      | Color    | Label       | Live dot |
    | --------------------------- | ------------- | -------- | ----------- | -------- |
    | Listening for sessions      | `listening`   | `green`  | Listening   | yes      |
    | Session joined              | `joined`      | `green`  | Joined      | yes      |
    | Session available to join   | `available`   | `blue`   | Available   |          |
    | Session ended               | `ended`       | `gray`   | Ended       |          |
    | Session gone from server    | `unavailable` | `yellow` | Unavailable |          |
    | Local recording in progress | `recording`   | `red`    | Recording   | yes      |
    | Local recording finished    | `completed`   | `green`  | Completed   |          |
    | Device streams to LiveKit   | `streaming`   | `green`  | Streaming   |          |
    | Device records only         | `recordOnly`  | `gray`   | Record only |          |

    Ended is gray, not red: it's a normal outcome. Red is reserved for live recording and errors.

- Upload progress: `Progressbar color="primary" size="h-4"` with `labelOutside="Upload progress"`.
- Long-running actions (registering, starting, joining a session, uploading) show a `Spinner` or `Progressbar` and disable the button that started them. For a spinner inside a filled button, use `<Spinner size="4" color="gray" class="me-2" />`.
- Errors from Rust (`publication-notification` failures and failed `invoke` calls) show as an `Alert color="red"` with an `ExclamationCircleOutline` icon and the reason text, never only in the console. Don't call SvelteKit's `error()` from event handlers: it throws uncaught and shows nothing.

## When to create a wrapper

Create a component in `src/lib/components/ui/` only when the same Flowbite component + props combination appears in 3+ places, or when it composes several Flowbite pieces into one reusable unit (e.g. `FormField` = Label + Input + Helper + error state). Wrappers:

- Take an explicit, typed `Props` interface (`variant`, `size`); no `...rest` passthrough.
- Use tokens only.

## Accessibility

- Every input has a `Label`; icon-only buttons have `aria-label`.
- Don't rely on color alone for status; pair it with an icon or text.
- Keep Flowbite's focus rings; never `outline-none` without a replacement.

## What the checker blocks

`pnpm check:design` will fail on:

- Hex / `rgb()` / `hsl()` colors in `.svelte` files
- Tailwind arbitrary values like `bg-[#fff]`, `mt-[13px]`, and CSS-variable shorthand like `bg-(--brand)`
- Non-token Tailwind palettes as classes (`blue-`, `purple-`, `slate-`, `red-`, `green-`, `mauve-`, `olive-`, `mist-`, `taupe-`, …); status colors go through Flowbite `color` props
- Inline `style="color: …"` / `background` declarations
- Flowbite `color="…"` props outside `primary`, `secondary`, `alternative`, `gray`, `dark`, `light`, `red`, `green`, `yellow`, `blue`, `none` (e.g. `color="purple"`)
- Hand-written `<svg>` in `.svelte` files (use `flowbite-svelte-icons`)
- Gradient backgrounds (`bg-gradient-*`, `bg-linear-*`, `bg-radial-*`, `bg-conic-*`)

It does not check layout, radius, shadows, or accessibility; review those by hand against this guide.

Add `<!-- design-ignore-next-line -->` (or `// design-ignore-next-line`) above a line only when there's a real reason, and say why.
