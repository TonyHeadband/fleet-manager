---
name: conventional-semver
summary: Conventional Commits titles, Summary and Test plan, semver from 1.0.
title: '^(feat|fix|docs|test|ci|chore|refactor|perf|build)(\([a-z0-9-]+\))?!?: \S.*$'
sections: [Summary, Test plan]
versioning: semver
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

This project follows semantic versioning:

| Change | Bump |
| --- | --- |
| Breaking | major |
| Feature | minor |
| Fix | patch |

Judge the bump from the change, not from the title alone.

## Release note

When the repo names a release-notes file and your breakdown planned a release note, add one line for this change
to that file in the same pull request.
