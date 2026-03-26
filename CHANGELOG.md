# Changelog

All notable changes to Reviu are documented here.

## 0.0.7

### macOS Status Bar for GitHub Notifications

Reviu now lives in your macOS menu bar. See your unread GitHub notification count at a glance and browse the latest notifications directly from the status bar dropdown — without switching to the app.

### Customizable Font Size

A new font size setting lets you scale the entire interface to your preference. Adjust the base size between 12 px and 24 px from Settings — every element scales proportionally in real time.

### Pin Favorite GitHub Repositories

Pin your most-used repositories to keep them at the top of the GitHub home list. Hover any repository row and click the pin icon to toggle. Pinned repos persist across sessions.

### macOS Native App Menu

Reviu now includes a native macOS menu bar with quick access to navigation (Git, GitHub, Git Config, Billing), Settings, About, and standard Edit actions (Undo, Redo, Cut, Copy, Paste, Select All).

### Stage, Unstage, and Restore Files Inline

You can now stage, unstage, or discard changes on individual files directly from the changes list on the Git page. Action buttons appear on hover for a faster, more precise staging workflow.

---

## 0.0.6

### In-App Feedback Dialog

Send bug reports and feature requests without leaving Reviu. A new feedback dialog lets you pick a type, enter a title and description, and submit directly to the team.

### GitHub Asset Image Rendering

Markdown content in pull request descriptions, comments, and reviews now renders GitHub-hosted images. Private asset URLs are resolved and signed automatically so images display inline.

### Pull Request Review Status Indicators

The pull request overview now shows clear visual indicators for each review state — approved, changes requested, commented, or dismissed — with color-coded icons and labels.

### Notification Count Badge

Your unread GitHub notification count is now visible on the user avatar and the macOS app dock icon, so you always know when something needs your attention.

### Notification Click Navigation

Clicking a notification navigates directly to the relevant page — pull request details, repository issues, or the GitHub web URL — and marks it as read automatically.

---

## 0.0.5

### Automatic App Updates

Reviu now checks for updates on launch. When a new version is available, a notification appears with a one-click download and install flow. Updates are verified with SHA-256 checksums before installation.

### Pull Request CI Checks

The pull request overview displays CI check status with a detailed summary — total, passing, failing, pending, and required checks — so you can review merge readiness at a glance.

### Pull Request Review Actions

Submit reviews directly from the pull request changes tab. Approve, request changes, or leave a comment with a review body — all without leaving the app.

### Merge Pull Requests

Merge pull requests from Reviu with support for merge commit, squash, and rebase strategies. The merge dialog validates readiness and lets you customize the commit title and message.

### GitHub Notifications Feed

A dedicated notifications tab on the GitHub home page shows your latest GitHub notifications grouped by repository, with unread indicators, search filtering, and a mark-as-done action.

---

## 0.0.4

### GitHub Repository Overview

Browse any GitHub repository directly in Reviu with an overview tab showing the README, description, language, stars, and forks.

### Pull Request Overview and Conversation

View pull request details with a full conversation timeline — description, comments, reviews, and replies — rendered with GitHub-Flavored Markdown.

### Pull Request Changes Diff

Review pull request file changes with inline and split diff view modes, syntax highlighting, and file-level navigation.

### Pull Request Commits List

Browse the list of commits in a pull request from the context sidebar, with search and filtering support.

### Editor Indent Rainbow

A new setting colors indentation guides by nesting level in the diff editor, making it easier to follow code structure at a glance.

---

## 0.0.3

### GitHub Integration

Connect your GitHub account to list your open pull requests and repositories needing review, all from the GitHub home page inside Reviu.

### Command Palette with Branch Operations

The command palette now supports switching branches, creating new branches, and creating branches from a base — all triggered with a keyboard shortcut.

### Settings Page with Theme Selection

A new settings page lets you toggle between dark and light themes, or enable auto-switching to follow your macOS system appearance.

---

## 0.0.2

### Advanced Git Operations via Command Palette

The command palette now handles merge, rebase, and interactive rebase workflows. Start, continue, skip, or abort a rebase directly from the palette. Merge conflicts are surfaced inline.

### Git Stash Support

Stash and restore your work-in-progress changes from the command palette. Browse your stash list, apply or drop entries, and keep your working tree clean between tasks.

---

## 0.0.1

### Local Git Workflow

Stage, unstage, and commit changes with a keyboard-first interface. Reviu handles the core Git operations you use every day — fetch, push, pull, and branch management.

### Repository and Branch Management

Add repositories to Reviu and switch between branches from the main Git page. The branch selector supports detached HEAD state and displays the current branch at all times.

### Diff Editor with Syntax Highlighting

View file diffs with inline and split modes, full syntax highlighting, and color-coded additions and deletions. The editor renders diffs in real time as you navigate the changes list.
