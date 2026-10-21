# UI localization

SolidCraft embeds its translations and works offline. English, Czech, Danish, German, English (UK), Spanish, French, Galician, Italian,
Japanese, Korean, Malay, Polish, Brazilian Portuguese, Russian, Turkish, Ukrainian and Chinese are currently registered in `crates/ui-egui/src/i18n.rs`. Adding a language requires an
embedded catalog and one registration there; the preferences selector and validation use the same
registry.

## Messages

`assets/locales/en.json` defines stable message IDs and English fallback text.
Keep IDs when changing wording. Other catalogs use the same IDs; missing messages fall back
to English. Use `app.tr("ui.preferences")` for static messages and
`Locale::format` for full sentences with named placeholders. Translate the entire sentence,
including punctuation, so a translator can reorder its arguments. Placeholder values are
inserted literally, even if a document name contains braces.

The English-label adapter (`app.tr_label`) supports existing engine and UI tables. It maps
display text to a message ID, then uses the same catalog lookup. New UI messages should use
stable IDs directly. The adapter does not change command IDs, model names, or scenario handles.
When a table's English wording changes, update the corresponding English catalog entry too.
Generated browser labels use named templates while their underlying model names stay intact.

Catalogs are parsed once and looked up through ordered maps. This is offline localization,
not a claim of zero runtime overhead. The catalog tests check registered locale IDs,
translation keys, and placeholder consistency. Run `cargo xtask ci` before submitting changes.

## Terms

The [glossary](book/src/reference/glossary.md) gives one English term for each idea, and each
entry has a note for translators: what the term must not be confused with. Its "Words with more
than one meaning" table lists the English words that need a separate catalogue entry for each
meaning (Construction, Flat, Round, Normal, Gap, Clearance).

## English (UK)

`en-GB.json` holds only the English messages whose spelling differs in UK English (Centre,
Customise, Organisation, Licence, colours, …); every other message falls back to English. When an
English message with a US spelling is added, add its UK spelling there too.

## Coverage

A message a locale never received falls back to English silently, so a gap shows up only as a row
that looks untranslated. Two guards catch that in the build:

- `every_complete_catalog_carries_every_english_message` — adding a key to `en.json`
  without giving it to a complete catalog fails the tests. English is the source catalog,
  Japanese and Chinese are delivered complete; Danish, German, Spanish, Galician, Italian, Korean, Malay, Polish, Brazilian Portuguese, Russian, French, Ukrainian and
  Turkish are partial — their gaps fall back to English and are reported by `cargo xtask i18n` without
  failing the build. A new label needs en, ja, zh, de and ko; Russian, French, Ukrainian and Malay may
  follow later.
- `every_toolbar_tab_and_panel_is_in_the_english_catalog` — tabs and panels reach the UI as bare
  source strings, resolved only by the English-label adapter, so a heading missing from `en.json`
  can never translate.

Command labels are the same case on a larger scale: every label in the engine's registry now has
an entry under `cmd.<command id>`, and `every_command_label_is_in_the_english_catalog` fails if a
command is added without one.

The context menu and the browser are the same case again, and they carry the most labels of all:
`every_menu_label_is_in_the_english_catalog` opens every menu the app can reach, flips the state
each conditional label branches on (origin shown, bodies locked, bodies hidden) so both branches
are seen, and fails on any label the catalog has never seen. Unit symbols are left out of the
sweep: `mm` is `mm` in every language, so they are not prose to translate. The sweep also asserts
it keeps reaching a broad set of labels, so it cannot pass by finding nothing.

`cargo xtask i18n` prints the same picture across every locale — which messages each one is
missing and which command labels English cannot translate. `--check` turns it into a non-zero exit
for CI. `--add-missing` writes each uncovered label into `en.json` under `cmd.<command id>`, with
the English label as its text, so the label adapter can find it and only the translations are left
to do. One key per distinct label: two commands that share a label (`Trim`, `Extend`) must not both
claim it, or the English-text lookup would silently pick one.

## Rendering and coverage

The Italian catalog uses original translations of SolidCraft's English messages, not text
extracted from commercial CAD software. Its core terminology is Schizzo (Sketch),
Estrusione (Extrude), Raccordo (Fillet), Smusso (Chamfer), Corpo (Body), Componente
(Component) and Vincolo (Constraint). Draft geometry is Sformo, while a report draft
is Bozza; Counterbore is Lamatura and Countersink is Svasatura. Established technical
terms such as Loft, Sweep, Spline and Offset remain unchanged. Command IDs, shortcuts,
file formats, units and user-provided names are not translated.

One immutable `Locale` type handles all translations. Table widgets without an app reference
use a render scope that restores the previous locale when it exits, including nested renders.
Standalone translated panels enter the same scope. Measure rendered text to size custom
widgets; UTF-8 byte lengths do not describe their displayed width.

The desktop host tries system CJK fonts across Windows, macOS and Linux, as well as an optional
`CRAFT_FONTS_DIR` font pack, and skips files that cannot be parsed. Chinese uses its own fallback
chain after the general CJK font: many Japanese CJK fonts (Hiragino Gothic, Meiryo, Yu Gothic)
match the CJK font search but lack simplified characters, so a second fallback found by the
Chinese font-name list (`PingFang`, `Microsoft YaHei`, `Noto Sans CJK SC`, …) covers them per
glyph. No fonts are redistributed in native desktop distribution packages; native builds rely
entirely on existing operating system fonts or craft-fonts.

In the browser, `solidcraft-web` bundles a self-hosted Japanese font (`BIZUDPGothic-Regular.ttf`, licensed
under SIL OFL 1.1) within the web distribution artifacts only. The font is loaded asynchronously from the same
origin (`./fonts/BIZUDPGothic-Regular.ttf`) on-demand when the interface language is a CJK one, or, with
any other interface language, the first time Japanese text (kana, kanji or full-width forms in a design name,
file name, body name or typed text) is drawn, without contacting any external CDNs or third-party servers.
A session that never shows such text downloads no font. If font loading fails with a CJK interface, the web
interface shows English for that session only to prevent missing glyphs (tofu); the saved language
preference is kept, so the next visit tries again. The bundled font specifically targets Japanese; it also
renders the kanji Chinese shares with Japanese, while Korean (Hangul) and simplified-only Chinese characters
are not covered, so a self-hosted SC/Korean font bundle remains follow-up work for full web coverage.

The current messages support named substitution, not locale-specific plural selection,
number/date formatting, or right-to-left layout and shaping. Add those capabilities when
introducing messages or languages that need them; use an established message formatter for
plural rules instead of adding language branches to panels. The catalogs cover the UI surfaces
introduced by this PR, not every engine error or every newer command dialog.

## Interactive tool diagnostics

`error.*` messages cover validation and selection feedback in interactive sketch tools,
feature dialogs and dimension editing. They intentionally exclude kernel/import diagnostics,
control-channel validation and scenario assertions. The two revolve-axis plane diagnostics
are included because they are shown directly while using Revolve.

`scripts/i18n_errors.py` inventories this scope from Rust syntax, skipping test code,
comments and logging. Install `scripts/requirements-i18n-errors.txt`, then use
`python scripts/i18n_errors.py --check` or `--add-missing`. The ignored report at
`target/error-messages.json` gives each template's source locations. This is scoped coverage,
not a guarantee that every error reachable from the engine has been localized.

The UI translates diagnostics at display time, in dialog error rows, status toasts,
message history and feature tooltips. Stored engine errors and issue-report input retain their
original English text. Templates use named fields; numeric formatting has already happened
before display, so the substituted text keeps its original values and units. Argument text is
inserted literally, including braces and user-supplied names. Unknown, malformed and ambiguous
templates retain their original diagnostic. Missing translations fall back to English.

Only the explicitly named `reason`, `error`, `msg` and `why` fields are interpreted as nested
diagnostics. Other arguments, such as command IDs and model names, are preserved. This is an
adapter for existing text errors, not a new engine error format. New code should prefer stable
message IDs and structured arguments over adding further formatted-text matching.

New English messages must also be translated in the complete Japanese and Chinese catalogs;
the existing completeness checks remain unchanged. Partial catalogs may add translations
incrementally. Do not fill a translated catalog with English solely to satisfy the checks.

## Adding tool descriptions on hover

The existing ribbon help uses `help.<command id>` messages and the sorted `DOCUMENTED`
list in `crates/ui-egui/src/command_help.rs`. To document a tool:

1. Verify its actual command ID in the engine registry.
2. Add the ID to `DOCUMENTED`, keeping the list sorted.
3. Add `help.<command id>` in `assets/locales/en.json`: two short sentences describing
   what the tool does and when to use it. Keep the command's name separate from the explanation.
4. Add the same key to the Japanese and Chinese catalogs, which require complete coverage.
   Add the Danish explanation in `da.json` when available; other missing translations fall
   back to English.
5. Run the command-help tests and localization checks. Toolbar buttons and panel menu rows
   already call the shared tooltip renderer; they do not need separate tooltip text.

For example, documenting `solid.loft` requires an entry named `help.solid.loft` plus
`solid.loft` in `DOCUMENTED`. Adding the JSON entry alone does not enable the tooltip.
This example describes the extension point; it does not add a Loft description.
