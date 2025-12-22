# Reviu Agents

Project specification file is located at: `./spec.md`

Phase progress tracking files are located at: `./PHASEX.md`

Don't create documentations files, only update existing ones `PHASEX.md` files and `spec.md` ONLY IF NEEDED.

## Desktop Application

The desktop application is located at: `./desktop`

For errors NEVER run the desktop application with `cargo run` instead use `cargo check` to identify errors.

Zed project is cloned at: `./zed`
YOU MUST use zed as a reference for building the desktop application.

gpui code examples:
- `zed/crates/gpui/examples`
- `zed/crates/ui/src`

## Backend

NodeJS + Hono server application.

The backend server is located at: `./backend`

To identify errors run `pnpm typecheck`
