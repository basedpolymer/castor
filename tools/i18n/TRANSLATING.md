# Translating Castor

Files: `locales/*.yml` (rust-i18n "version 2": each key holds every language). English (`en`) is
the reference; French (`fr`) is done by hand. Languages to add, in this order under each key:
`de`, `es`, `it`, `pt-BR`, `ja`, `zh-CN`, `ko`.

## Rules

- Keep keys, order, comments and the `en`/`fr` lines untouched. Add the 7 languages right after
  `fr`, same indentation (two spaces).
- Placeholders `%{name}` stay exactly as they are (same names, same count). Keep `→`, `·`, `×`,
  units (KB, MB, dpi, px, fps, LUFS), file formats (PDF, JPG, WebP…), keyboard keys, and
  product names (Castor, Hangar, GitHub, ffmpeg, Whisper, Typst, MuPDF, Magika…) unchanged.
- Quote values with double quotes when they contain `:`, `#`, `%{`, leading/trailing spaces or
  start with a special YAML character; escape inner `"` as `\"`.
- Short UI labels stay short (buttons, segmented choices, tabs): aim for ≤ 130 % of the English
  length; never add a final period to labels; descriptions keep their sentence punctuation.
- Register: de **Sie**; es **tú**; it **tu**; pt-BR **você**; ja **です・ます**; zh-CN neutral,
  no 您 except in polite requests; ko **합니다/하세요** polite.
- Use the terminology of the OS UI in each language (the words macOS and Windows use for
  file, folder, settings, menu bar, system tray, clipboard, drag and drop, undo…).
- Typography: de „…“; es/it/pt-BR « » or “ ” as usual in software (prefer “ ” for es/pt-BR,
  « » for it is acceptable — be consistent per file); ja/zh-CN full-width punctuation 、。「」；
  ko normal spacing. French rules do not apply to other languages.
- Humour lines (`*.done`, empty states) should stay light and dry; adapt rather than translate
  literally, keep the beaver (Castor = beaver) jokes where they work.

## Glossary (English → keep consistent across files)

run (a job, noun) · chain · preset · output · queue · studio · favorite · workspace ·
quick panel · menu bar (macOS) / system tray (Windows) · drop zone · compress · convert ·
redact · watermark · stamp · trim · split · merge/join · transcribe · captions/subtitles ·
background removal · upscale · OCR (keep "OCR") · metadata · checksum.

## Check before finishing

`py -I tools/i18n/check_locales.py` (parses every file, verifies 9 languages per key and identical
placeholders) — add it if missing.
