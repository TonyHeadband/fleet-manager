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
