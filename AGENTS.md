# Reviu Agents

You can use osgrep "search query" to search on the codebase

Always use Context7 MCP when I need library/API documentation, code generation, setup or configuration steps without me having to explicitly ask.

I have a repo git-playground, I use it to test the git related features: /Users/joris/workspace/git-playground/

## GPUI Desktop app

Framework: Gpui

Always run cargo check for errors: cd ./desktop/ && cargo check
Add tests when finishing a feature or fix
For icons use gpui-components icons

Gpui tips:

- For on_click and overflow div need and id

Gpui examples:

- Gpui examples are available at ./desktop/gpui
- Gpui-components examples are available at ./desktop/gpui-components

## Nodejs backend

Framework: Hono
Auth: better-auth (Github)

Always run type check for errors: cd ./backend/ && pnpm typecheck

Auth open api swagger file: http://localhost:3000/api/auth/open-api/generate-schema
