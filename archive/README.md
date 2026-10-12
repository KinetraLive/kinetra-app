# Archive

Features taken out of the app and kept here, unchanged, so they can come back. Nothing in this
folder is built, embedded or tested: presets are only embedded from `presets/`, and the app's
routes only from `src/routes/`.

## Chat

- `chat/route/+page.svelte`: the Chat page (`/chat`), an empty state; live chat itself was never
  built.
- `presets/templates/chat/`: the Chat preset (all style looks, `sample.json`).
- `presets/thumbs/chat-*.webp`: its gallery thumbnails.
- `e2e-screenshots/*chat*`: its screenshot baselines.

Removed with it: the Chat item in the sidebar, the `chat` route scope for shortcuts ("In Chat")
and `{ "id": "chat", "kind": "chat" }` in `presets/library.json`.

Still in the app (needed when chat returns): the `chat-themes/` project folder in backups and the
settings export, and the row-mark plumbing in `core/fieldvals.rs`.

## Restoring

Move the folders back (the preset to `presets/templates/chat`, the thumbnails to
`presets/thumbs/`, the baselines to `e2e/__screenshots__/`), add its line back to
`presets/library.json` and the preset test lists, and restore the route, the sidebar item and the
shortcut scope.
