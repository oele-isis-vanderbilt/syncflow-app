#!/usr/bin/env node
/**
 * Design-system checker for src/.
 *
 *   pnpm check:design                    → scan all of src/
 *   node scripts/check-design.mjs a.svelte → scan specific files
 *   (Claude Code PostToolUse hook)       → reads the edited file path from the
 *                                          hook JSON on stdin; files outside
 *                                          src/ are ignored
 *
 * Exit 0 = clean. Exit 2 = violations (Claude Code shows stderr to Claude so it
 * fixes them in the same turn).
 *
 * Suppress a line with a preceding `<!-- design-ignore-next-line -->` or
 * `// design-ignore-next-line` comment, and say why.
 */
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { join, extname, relative, resolve, dirname, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const WEB_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SRC_DIR = join(WEB_ROOT, 'src');
const SCAN_EXT = new Set(['.svelte', '.ts', '.js']);
const SKIP = [/\.d\.ts$/, /\.(test|spec)\.[jt]s$/];

// Tailwind palettes that are NOT design tokens. Status colors (red/green/yellow)
// are allowed only via Flowbite `color` props, so they're not listed as classes either.
const FOREIGN = [
    'slate',
    'zinc',
    'neutral',
    'stone',
    'red',
    'orange',
    'amber',
    'yellow',
    'lime',
    'green',
    'emerald',
    'teal',
    'cyan',
    'sky',
    'blue',
    'indigo',
    'violet',
    'purple',
    'fuchsia',
    'pink',
    'rose',
    // added to Tailwind's default theme in v4.2
    'mauve',
    'olive',
    'mist',
    'taupe',
];
const UTIL =
    '(?:bg|text|border|ring|outline|fill|stroke|from|via|to|divide|placeholder|decoration|accent|caret|shadow)';

const RULES = [
    {
        id: 'raw-color',
        re: /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|oklch)\(/g,
        msg: 'Raw color value. Use a token class (primary-*, secondary-*, gray-*).',
        exts: ['.svelte'],
    },
    {
        id: 'arbitrary-value',
        // `p-[13px]`, `bg-[#123]`, and v4's CSS-variable shorthand `bg-(--brand)`
        re: /\b[a-z][a-z0-9-]*-(?:\[[^\]\s]+\]|\(--[^)\s]+\))/g,
        msg: 'Tailwind arbitrary value. Use the default scale, or ask to add a token.',
    },
    {
        id: 'foreign-palette',
        re: new RegExp(`(?:[a-z-]+:)*\\b${UTIL}-(?:${FOREIGN.join('|')})-\\d{2,3}\\b`, 'g'),
        msg: 'Non-token palette. Use primary-*/secondary-*/gray-*; status colors via Flowbite color props.',
    },
    {
        id: 'inline-style-color',
        re: /style=["'{][^"'}]*\b(?:color|background(?:-color)?)\s*:/g,
        msg: 'Inline color styling. Use token classes instead.',
        exts: ['.svelte'],
    },
    {
        id: 'flowbite-color-prop',
        // Flowbite `color` props accept every Tailwind palette; only tokens and status colors are allowed.
        re: /\bcolor=["'](?!(?:primary|secondary|alternative|gray|dark|light|red|green|yellow|blue|none)["'])[^"']*["']/g,
        msg: 'Off-system Flowbite color prop. Use primary/secondary/alternative/gray, or red/green/yellow/blue for status.',
        exts: ['.svelte'],
    },
    {
        id: 'inline-svg',
        re: /<svg\b/g,
        msg: 'Hand-written SVG. Use an icon from flowbite-svelte-icons.',
        exts: ['.svelte'],
    },
    {
        id: 'gradient',
        re: /\bbg-(?:gradient|linear|radial|conic)-[a-z0-9-]+/g,
        msg: 'Gradients are not part of the design system. Use a token surface (bg-white, bg-gray-50).',
    },
];

function walk(dir, out = []) {
    for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) walk(p, out);
        else if (SCAN_EXT.has(extname(p))) out.push(p);
    }
    return out;
}

function inScope(file) {
    return (
        file.startsWith(SRC_DIR + sep) &&
        SCAN_EXT.has(extname(file)) &&
        !SKIP.some((r) => r.test(file))
    );
}

function check(file) {
    const ext = extname(file);
    const lines = readFileSync(file, 'utf8').split('\n');
    const found = [];
    lines.forEach((line, i) => {
        if (i > 0 && lines[i - 1].includes('design-ignore-next-line')) return;
        for (const rule of RULES) {
            if (rule.exts && !rule.exts.includes(ext)) continue;
            for (const m of line.matchAll(rule.re)) {
                found.push(
                    `${relative(process.cwd(), file)}:${i + 1}  [${rule.id}] "${m[0]}"  ${rule.msg}`
                );
            }
        }
    });
    return found;
}

// The hook writes its JSON immediately. A non-TTY stdin that stays silent (CI, a
// subshell with an open pipe) means a plain run, so stop waiting after a short delay.
function readStdin(timeoutMs = 500) {
    return new Promise((done) => {
        let raw = '';
        const finish = () => {
            clearTimeout(timer);
            process.stdin.destroy();
            done(raw);
        };
        const timer = setTimeout(() => {
            if (!raw) finish();
        }, timeoutMs);
        process.stdin.setEncoding('utf8');
        process.stdin.on('data', (chunk) => (raw += chunk));
        process.stdin.on('end', finish);
        process.stdin.on('error', finish);
    });
}

async function hookFiles() {
    if (process.stdin.isTTY) return null;
    const raw = await readStdin();
    if (!raw.trim()) return null;
    try {
        const input = JSON.parse(raw);
        const p = input?.tool_input?.file_path;
        if (!p) return [];
        return [resolve(input.cwd ?? process.cwd(), p)];
    } catch {
        return null;
    }
}

const args = process.argv.slice(2).map((f) => resolve(f));
const fromHook = args.length ? null : await hookFiles();
let files = args.length ? args : (fromHook ?? (existsSync(SRC_DIR) ? walk(SRC_DIR) : []));
files = files.filter((f) => existsSync(f) && inScope(f));

const violations = files.flatMap(check);
if (violations.length) {
    console.error(
        `Design system violations (${violations.length}). See docs/design-rules.md:\n\n` +
            violations.join('\n')
    );
    process.exit(2);
}
if (!fromHook) console.log(`✓ Design check passed (${files.length} files)`);
