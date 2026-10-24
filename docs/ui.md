# How the UI is put together

The UI draws whatever the daemon's `UiSchema` describes (`GET /api/ui-schema`; defined in `crates/fleet-api/src/ui.rs`). Components hold no status, column or action names. Add or change what the board shows in the schema first; touch React only for a new kind of thing.

## Layers

| Layer | Where | Job |
|---|---|---|
| Schema logic | `web/src/api/schema.ts` | `evalCondition`, `columnFor`, `visibleActions`, `actionRequest`, `fieldValue`, `formatDuration`. Pure functions, no React. |
| State | `web/src/api/board.ts`, `events.ts`, `hooks.ts` | `boardReducer` applies events; `subscribe` and `useFleet` feed it from SSE. |
| Primitives | `web/src/ui/` | `Button`, `Badge`, `Tabs`, `Field`. The only place a raw `<button>` or a badge `<span>` is written. |
| Components | `web/src/components/` | `Board` → `Column` → `Card`, plus `RepoFilter`, `SpawnForm`, `PreviewPane`. They compose primitives and read the schema. |
| Styles | `web/src/styles.css` | The only stylesheet. Colours are variables on `:root` (with a dark-mode override); tone and variant come from `data-tone`, `data-variant` and `aria-pressed` attributes. |

## Rules

- A new card field is a `FieldDef` in the schema. `Field` draws it by kind (`text`, `badge`, `duration`, `mono`) and renders a list value as one element per entry.
- A new button action is an `ActionDef` in the schema. `client.run` executes it; no component names an action.
- A new column is a `ColumnDef`. `Board` renders it with `Column`.
- `fieldValue` derives exactly two keys from the workspaces: `repos` and `branch`. A contract test in `fleet-api` rejects any other card field that is not an `Agent` field.
- Use `Button` and `Badge`, never a raw `<button>` or a `.badge` span. Style a new look as a `data-variant` in `styles.css`, not as a new class.
- No inline `style=` props and no per-component stylesheets.
- Inputs, selects and textareas are plain elements styled by element selectors in `styles.css`; they have no behaviour of their own to share.

## Tests

`pnpm --dir web test`. Component tests query by role and title, never by class, so a restyle does not break them. The default schema the tests use is `web/src/api/generated/default-ui-schema.json`, written by `cargo test -p fleet-api`, so the UI is always tested against the daemon's real default.

## egui client (`crates/fleet-gui`)

- `theme.rs` is the only place for colours, type sizes, corner radii and fonts. Light and dark come from one token set; the rest of the crate reads tokens and never writes a literal colour.
- Spacing (margins, gaps, padding) is not tokenised: each component sets its own, next to the layout it belongs to.
- `widgets/` is the only place buttons, badges, tabs, icons and dots are drawn. A new look is a variant of a primitive there, not a styled one-off in a component.
- egui's stock widgets (`ComboBox`, `checkbox`, `TextEdit`, `ScrollArea`, tooltips) are not redrawn in `widgets/`: they pick up the tokens through the `Visuals` that `theme::apply` installs, and components call them directly.
- Components in `components/` compose those primitives and read the schema. They never write a colour or a font size.
- The fonts (IBM Plex Sans Regular and Medium, IBM Plex Mono Regular) are vendored under `crates/fleet-gui/assets/fonts/` with their SIL OFL licence and compiled in with `include_bytes!`, so the binary runs air-gapped.
- The window must stay idle: no repaint loop beyond the 15 s refresh, events and background results. Motion goes only through `theme::animate(ctx)` (with `theme::pulse` built on it): it is true only while the window has focus, and then frames are throttled to about 100 ms (`theme::ANIMATION_FRAME`). It is false while the window is unfocused and in every test (`egui_kittest`'s `Harness::run()` panics on continuous repaints).
- Tests (`cargo test -p fleet-gui`) find widgets by role and label, so a restyle does not break them; keep visible labels unchanged.
