---
name: conventional
summary: Conventional Commits titles, Summary and Test plan, no version note.
title: '^(feat|fix|docs|test|ci|chore|refactor|perf|build)(\([a-z0-9-]+\))?!?: \S.*$'
sections: [Summary, Test plan]
versioning: none
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

## Release note

When the repo names a release-notes file and your breakdown planned a release note, add one line for this change
to that file in the same pull request.
