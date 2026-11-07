# Actions

One registry for everything a user can invoke: editor actions, shell built-ins, native and
WASM extension commands. Menus, toolbar, keyboard, hotkeys dialog and the command palette are
views over it; none of them contains action-specific code. Designed for thousands of installed
extensions: listing an action never activates its extension.

## Model

```text
Action
  id            "<owner>.<name>"      owner = extension id ("exaudio.core" for the shell/editor)
  title         { en, ru, … }         shown in menus, palette, hotkeys dialog
  category      { en, ru, … }         palette group / hotkeys section; defaults to the owner's name
  when          When                  enablement; false → hidden everywhere, key does not fire
  palette       bool (default true)   listed in the palette
  keys          [ { key, when?, mac? } ]   default bindings; user overrides replace them
  menus         [ { menu, group, order, when? } ]   "editor/overflow", "editor/toolbar", "row/context", …
  handler       bound separately (see Binding)
```

`id` MUST start with the owner's id followed by `.`; the registry rejects foreign prefixes, so
two extensions cannot collide and an id names its owner without a lookup.

### When clauses

A small expression language evaluated against a context snapshot:

```text
expr   := or
or     := and ( "||" and )*
and    := not ( "&&" not )*
not    := "!" not | atom
atom   := "(" expr ")" | key | key ("==" | "!=") value
key    := [a-zA-Z_][a-zA-Z0-9_.-]*
value  := 'quoted' | true | false | number
```

Parsed once at registration (invalid clause → registration error). Context keys the shell sets:

| key | values |
|---|---|
| `editorOpen` | bool |
| `focus` | `workspace` · `transcriptText` · `dialog` |
| `cutPreview` | bool (a cut-preview overlay exists) |
| `textSelection` | bool (focused row has a non-empty selection) |
| `mode` | id of the launched mode (`exaudio.local`, `exaudio.babel`, …) |
| `unsavedChanges` | bool |

Extensions set their own keys (`exaudio.babel.hasClaim`) through `Context::set`; keys are
namespaced by owner like action ids. Reading unknown keys yields `false`.

### Binding

Metadata comes from the manifest (`contributes.actions`) and is loaded for every installed,
enabled extension at startup without activating it. Handlers are bound when the owner
activates: `registry.bind(id, handler)`. Dispatching an unbound action activates the owner
first (`ActivationHook`), then retries once; still unbound → "This command isn't available."

Native extensions declare the same manifest JSON (`include_str!`) and bind in `register`.
Editor actions are declared by the shell from `ACTION_DEFS` (`exaudio.core.<kebab-id>`), with
`when` derived from `KeyContext`:

| `KeyContext` | `when` |
|---|---|
| Global | — |
| Timeline | `focus == 'workspace'` |
| CutPreview | `focus == 'workspace' && cutPreview` |
| TranscriptText | `focus == 'transcriptText'` |
| Transcript | `focus != 'dialog'` |
| Dialog | `focus == 'dialog'` |

## Registry

```rust
pub struct ActionRegistry {
    actions:  BTreeMap<ActionId, Entry>,              // deterministic iteration
    by_owner: HashMap<OwnerId, Vec<ActionId>>,
    keys:     HashMap<KeyChord, SmallVec<[BindingRef; 2]>>,   // resolved chord → candidates
    menus:    BTreeMap<MenuId, Vec<MenuRef>>,         // sorted by (group, order, id)
    search:   Vec<SearchRow>,                          // lowercased title+category per locale
    user:     UserBindings,                            // overrides keyed by ActionId
}
```

Invariants:

- Registration is atomic per owner: all contributions validate, then all indices update, or
  nothing changes. `unregister_owner` removes every entry and index row of that owner.
- Every index row references an existing `Entry`; tests assert index consistency after any
  mutation sequence.
- Key resolution is `O(candidates for that chord)`, never a scan: snapshot the context once per
  key event, look up the chord, keep candidates whose binding `when` and action `when` hold and
  whose owner is enabled, then pick by precedence: user override › `exaudio.core` › extension,
  ties by `order` then id. Shadowed candidates are reported as conflicts to the hotkeys dialog.
- Palette filtering scans `search` (one lowercase row per action); at 10⁴ actions this is well
  under a millisecond. Rows are rebuilt only on registration, locale change or title change.

### User overrides

`exaudio.keymap` in UI storage:

```json
{ "bindings": [ { "action": "exaudio.core.split-at-playhead", "key": "ctrl-shift-s", "when": "focus == 'workspace'" } ],
  "removed":  [ "exaudio.babel.magic-review" ] }
```

Keyed by action id, so extension actions are rebindable like editor ones. The previous
`{ "<EditorAction id>": "<key>" }` file is migrated on first load by prefixing `exaudio.core.`.

## Surfaces

| surface | query |
|---|---|
| `⋯` menu | `registry.menu("editor/overflow", &ctx)` |
| toolbar buttons | `registry.menu("editor/toolbar", &ctx)` |
| row context menu | `registry.menu("row/context", &ctx)` (receives the row's segment id as an argument) |
| keyboard | `registry.resolve(chord, &ctx)` → `dispatch(id)` |
| hotkeys dialog | `registry.actions()` grouped by category; rebinding writes `user` |
| command palette | `registry.actions_in(&ctx).filter(palette)` + search |

Dispatch is `shell.run(id, args, window, cx)`; arguments are JSON (`{"segment_id": …}`), the
same for native and WASM handlers.

## Extension API

`exaudio-extension` gains `contributes.actions` in the manifest and the `When` parser (shared
with the index validator, so a catalog entry with an invalid clause is rejected at review
time). The WIT world gains `run-action(id, args)` and `set-context(key, value)`; the component
`on-action(id, args)` export replaces the per-command callbacks.

## Migration

1. `exaudio-extension`: `ActionManifest`, `When` (parser + evaluator), tests.
2. `exaudio-shell`: `ActionRegistry` with indices and user bindings; core actions from
   `ACTION_DEFS`; `EditorScreen::on_key_down` resolves through it; `ActiveKeymap` becomes the
   `user` part of the registry; hotkeys dialog, `⋯` menu, toolbar read the registry.
3. Callers: `babel-ui`, `gold-ui`, `local`, `wasm_ui` declare actions in their manifests and
   bind handlers; `CommandContribution`, `KeybindingContribution`, `register_command`,
   `register_keybinding`, `commands()` are deleted.
4. `exaudio.palette` extension: the first surface with zero knowledge of specific actions.
5. SDK, index validator and docs updated; the example extension declares an action.
