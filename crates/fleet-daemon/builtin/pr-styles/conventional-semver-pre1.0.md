---
name: conventional-semver-pre1.0
summary: Conventional Commits titles, Summary and Test plan, semver before 1.0.
title: '^(feat|fix|docs|test|ci|chore|refactor|perf|build)(\([a-z0-9-]+\))?!?: \S.*$'
sections: [Summary, Test plan]
versioning: semver-pre1.0
---
## Title

Conventional Commits: `<type>(<scope>): <what changed>`, the scope optional. The type is one of `feat`, `fix`,
`docs`, `test`, `ci`, `chore`, `refactor`, `perf` or `build`. Add `!` before the colon for a breaking change. The
description is lowercase, in the imperative, and says what the change does for its user, not how.

## Summary

Two to four sentences: what changed and why. Lead with the reason; the diff already shows the how.

## Test plan

What you ran and what it showed: the commands, the tests added, anything you checked by hand. Name what you could
not check.

## Version note

Put one line in the body when the change is breaking or touches a path the repo marks as breaking:

`Version: <major|minor|patch> — <why>`

This project is before 1.0 (`0.y.z`), so the counting is shifted down one place:

| Change | Bump |
| --- | --- |
| Breaking | minor |
| Feature | patch |
| Fix | patch |

Judge the bump from the change, not from the title alone. When the project reaches 1.0, it moves to the
`conventional-semver` style.

## Release note

When the repo names a release-notes file and a user would notice the change, write its release-notes line and quote
it in the brief of the step you mark `release_note: true`. That step's specialist adds the line to the file.
