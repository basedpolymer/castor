# Castor — design

Castor looks like a well-typeset LaTeX document that happens to be an app: the TeXtured
template (github.com/jdujava/TeXtured) and basedpolymer.eu/nutrition are the references.
Calm paper, black ink, numbered sections, captions, thin rules, one diamond motif. Colour is
rare and pale, used only to tell tool families apart. It must still feel like a fast, native,
modern tool: dense where it helps, generous where it calms.

## 1. Tokens

| token          | light      | dark       | use |
|----------------|------------|------------|-----|
| paper          | `#fcfbf9`  | `#161514`  | window background |
| surface        | `#f4f2ee`  | `#1e1d1b`  | sidebar, panels, inputs |
| raised         | `#ffffff`  | `#242321`  | cards, popovers, menus |
| ink            | `#1b1616`  | `#e8e6e1`  | text, primary buttons, selected segments |
| muted          | `#5c5a55`  | `#a6a39b`  | secondary text, captions |
| faint          | `#8c8a84`  | `#75726b`  | placeholders, disabled |
| rule           | `#d9d6cf`  | `#3a3833`  | 1 px borders, table rules |
| rule-strong    | `#1b1616`  | `#e8e6e1`  | booktabs top/bottom rules (1.2 px) |
| link           | `#2f6db5`  | `#8db4ea`  | links, focus ring |
| good           | `#2e7d4f`  | `#6cc28f`  | success, size saved |
| warn           | `#a8791a`  | `#e0b652`  | warnings |
| bad            | `#c0392b`  | `#f4897b`  | errors, destructive |

Family tints (TeXtured badge pastels) — card swatch, hover wash, tag background:

| family | light     | dark      |
|--------|-----------|-----------|
| PDF    | `#fbe2ff` | `#3b2840` |
| Image  | `#ccfaff` | `#1d3a3d` |
| Video  | `#ade1ff` | `#1c3046` |
| Audio  | `#aafaba` | `#1d3b24` |
| Create | `#ffeca4` | `#3b3417` |
| Files  | `#ffdcc8` | `#3e2a1f` |

Implement as a custom GPUI Kit theme (both modes) + a `castor_theme` module exposing the extra
tokens; follow Appearance setting (System/Light/Dark).

## 2. Type

Fonts are bundled (`assets/fonts`, GUST Font License) and registered at startup.

| role                 | font                                   | size / weight |
|----------------------|----------------------------------------|---------------|
| UI text, controls    | Latin Modern Sans 10 (`lmsans10`)      | 14 px regular |
| Section titles       | Latin Modern Sans 10 Bold              | 20 px, "1.  PDF" with the number in ink |
| Page titles          | Latin Modern Sans 10 Bold              | 30 px |
| Ghost initial        | TeX Gyre Chorus (calligraphic)         | 110 px, ink at 10 % opacity, behind the page title's first letter, offset up-left (like the "N" of /nutrition) |
| Descriptions, prose  | Latin Modern Roman 10 (`lmroman10`)    | 15 px |
| Captions             | Latin Modern Roman 10, "**Figure 1** / …" with the label bold | 13.5 px, muted |
| Emphasis             | Latin Modern Roman Slanted             | — |
| Numbers in tables    | Latin Modern Sans (tabular figures if available) or Mono | 13.5 px |
| Code, paths          | Latin Modern Mono 10                   | 13 px |

Latin Modern has a small x-height: never go below 13 px. Use real typographic characters:
“ ” ‘ ’ – — … × → ≈ (and French spacing rules in French strings: espace fine insécable before
; : ! ? and inside « »).

## 3. Motifs

- **Diamond ◇** (a square rotated 45°, 1.5 px stroke): the brand mark, bullets of section headers,
  the end of the page rule, the "current step" marker, the tray icon.
- **Page rule**: top-right of each page, a thin rule ending in a small diamond, with a number
  above it (tools count, runs count…) — like the page number of /nutrition.
- **Numbered sections**: "1.  PDF", "2.  Image"… on the tools home; "1. Options" / "2. Output"
  in the tool screen; Settings is "1. General", "2. Menu bar"…
- **Captions**: results and previews get "Figure n / …" captions; tables get
  "Table n — …" above them.
- **Booktabs tables**: strong rule on top and bottom, thin rule under the header, no vertical
  lines, generous row height (32 px), right-aligned numbers.
- **Buttons**: rectangular, radius 3 px, 1 px rule border on raised. Primary = ink fill, paper
  text. Destructive = bad text, no fill. Segmented controls: ink fill on the selected segment
  (like "masse | kcal" on /nutrition).
- **Checkboxes**: square, 1 px ink border, ✓ in ink.
- **Icons**: Lucide (bundled with GPUI Kit), 16 px, 1.5 px stroke, ink or muted.
- **No shadows** except popovers/menus (one soft shadow), **no gradients** in chrome.
- **Motion**: 120–160 ms ease-out on hover/selection; progress bars animate width; nothing bounces.

## 4. Layout

Main window: default 1180 × 760, minimum 880 × 560.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ ● ● ●   ◇ Castor        [ Search tools, or describe what you want…  ⌘K ]   ⚙ │  title bar
├───────────────┬──────────────────────────────────────────────────────────────┤
│ Tools         │                                                   86          │
│ Studios       │   𝒯 Tools                                ─────────────◇       │
│ Chains        │   ┌ Drop files here, or pick a tool. Nothing leaves this ┐    │
│ Runs          │   │ machine.                         [Choose files…]       │    │
│               │   └──────────────────────────────────────────────────────┘    │
│ FAVORITES     │   All · PDF 25 · Image 25 · Video 14 · Audio 12 · Create · Files│
│ ◇ Compress PDF│                                                                │
│ ◇ Convert img │   1.  PDF                                                      │
│ …             │   ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐                  │
│               │   │◆ Compr…│ │◆ Merge │ │◆ OCR   │ │◆ Redact│   cards 4–5/row  │
│               │   └────────┘ └────────┘ └────────┘ └────────┘                  │
│ ⚙ Settings    │   2.  Image …                                                  │
│ v1.0.0  ● upd │                                                                │
└───────────────┴──────────────────────────────────────────────────────────────┘
```

- **Sidebar** 216 px, surface colour. Items: Tools, Studios, Chains, Runs; "Favorites" group
  (tools, reorderable); bottom: Settings and version (with a dot when an update is ready).
- **Tool card**: 1 px rule border on raised, 12 px padding, family diamond swatch, name (sans
  bold 14.5), one-line description (roman 13.5 muted, 2 lines max), bottom row: input → output
  tag in mono-ish small caps ("PDF → PDF"), star on hover. Hover: family tint wash.
  After a drop: cards that take the files get an ink outline + "3 files" badge; the others fade
  to 40 % with "Not for these files".
- **Tool screen**: breadcrumb "Tools / PDF / Compress PDF"; title with ghost initial; the
  description in roman. Two columns: **files** (left, flexible) and **options** (right, 340 px).
  - Files: drop zone when empty; else a booktabs table: thumbnail 36 px, name, size, status
    (queued / progress bar / ✓ result size + change in good colour / ✗ error in bad colour),
    row actions (open, show in folder, remove). "Add files…" and "Clear" under the table.
  - Options: "1. Options" generated from the schema (see §6), "More options" disclosure for
    `advanced`, "2. Output" (next to originals / folder, one line + "Change"), then the primary
    button "Compress 3 files" (verb from `{id}.verb`, fallback "Run") and, while running,
    "Cancel".
  - Tools with no input (Create) and tools with `preview`: the left column shows the live
    preview with its caption instead of (or above) the file table.
  - When done: summary line "3 files · 32 MB → 9.8 MB (−69 %) · 4.2 s" + "Show in folder";
    image/PDF results get a compare view (side by side or swipe slider, Fit / 100 %).
- **Runs**: Table "Table 1 — Runs" grouped by day; filters All / Complete / Failed / Cancelled;
  row = tool or chain, files, size change, duration, age, status; click → details (outputs,
  open, show, run again with the same settings, undo for in-place tools).
- **Chains**: templates + user chains; editor lists steps as "Step 1 ◇ Resize → Step 2 ◇
  Compress", each with the same generated options form; add/remove/reorder; run on files or a
  folder; export/import TOML.
- **Settings**: a document — 1. General, 2. Menu bar (System tray on Windows), 3. Models &
  helpers, 4. Updates, 5. Privacy, 6. About.

## 5. Quick panel (menu bar / tray)

Click on the tray icon (or the global shortcut, default ⌘⌥Space / Ctrl+Alt+Space) opens a
360 × 520 borderless panel anchored under the icon (macOS) / above it (Windows), closed on
blur or Esc:

1. Search field (focused): type → tools; Enter opens the tool in the main window.
2. Drop zone "Drop files on me": after a drop, the 6 best tools for those files; clicking one
   runs it right away with the last-used values, progress inline, then "Show" / "Copy".
3. Clipboard line when the clipboard holds an image or files: "Clipboard: PNG 1920 × 1080 —
   Compress · Remove background · To JPG": the result goes back to the clipboard.
4. Favorites (one click = open tool, or run on dropped files).
5. Last 3 runs. Footer: "Open Castor", "Quit".

## 6. Generated controls

| `OptionKind`            | control |
|-------------------------|---------|
| Choice ≤ 4              | segmented control |
| Choice > 4              | select |
| Toggle                  | checkbox with label (switch only in Settings) |
| Int/Float `slider`      | slider + small number field + unit |
| Int/Float               | number field + unit |
| Text / multiline        | input / textarea |
| Password                | masked input with reveal eye; never persisted |
| Color                   | swatch + hex field + picker popover |
| Path                    | read-only path + "Choose…" (native dialog) |
| PageRange               | input, placeholder "All pages — e.g. 1-3, 5, 8-" |
| Anchor                  | 3 × 3 grid of small diamonds, selected filled |
| Size                    | "W × H" two number fields + lock ratio |
| Font                    | select of installed families, rendered in their own font |
| Language                | searchable select (multi = chips) |

Labels: `{id}.opt.{key}` (or `common.opt.{key}`), help text under the control in roman muted.
Hidden when `visible_if` is false. Values persist per tool (`Settings::tool_values`).

## 7. Words

- Plain, short, friendly; verbs on buttons ("Compress 3 files", not "Run").
- Sizes always "before → after (−69 %)".
- Errors say what happened and what to do, never a stack trace (details in a disclosure).
- Humour: rare and dry — empty states, the end of a run, About. Examples:
  - Runs, empty: "No runs yet. The beaver is resting."
  - Done, big saving: "19 MB → 212 KB. Neatly gnawed."
  - About: "Castor builds things. It does not fly planes." / "No accounts, no cloud, no beaver
    was harmed."
  - Model download: "Fetching the model once — after that it works offline."
  Never joke in errors or anything that touches the user's files.
