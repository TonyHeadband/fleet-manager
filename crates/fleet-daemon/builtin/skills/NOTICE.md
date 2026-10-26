# Notice

The skills in this folder are copied from **superpowers 7.0.0** by Jesse Vincent
(https://github.com/obra/superpowers), MIT License; see `LICENSE` in this folder. They are pinned to that
version: an upgrade is a new copy, reviewed and edited the same way.

Copied: `SKILL.md` of `brainstorming`, `writing-plans`, `test-driven-development`, `systematic-debugging`,
`verification-before-completion` and `receiving-code-review`, plus the reference files they link to:
`test-driven-development/writing-good-tests.md`; `systematic-debugging/root-cause-tracing.md`,
`defense-in-depth.md`, `condition-based-waiting.md`, `condition-based-waiting-example.ts`, `find-polluter.sh`.

Not copied: `brainstorming/visual-companion.md`, `brainstorming/builder-check-prompt.md`, `brainstorming/scripts/`,
`systematic-debugging/CREATION-LOG.md`, `systematic-debugging/test-*.md`.

## Edits

Fleet agents have no in-chat subagent tool and no task-list tool, and launch work only through the fleet daemon,
so the copies name only tools these agents have.

`brainstorming/SKILL.md`
- "Show, don't tell": the visual companion (a browser tab) is replaced by sketching in plain text in the terminal.
- "Spike to feel things out": a spike is a step of its own in the breakdown.
- "Play It Back": removed the check that sends the visual companion offer before a chunk.
- "Size the Work" and the hard gate: `superpowers:writing-plans` becomes "the writing-plans skill".
- "Builder check": the fresh subagent running `builder-check-prompt.md` is replaced by the orchestrator reading
  its own design once as its builder, with that prompt's instructions folded in; removed the "without a subagent
  tool" fallback line, now the only path.
- Removed the "Visual Companion" section.

`writing-plans/SKILL.md`
- Removed the `superpowers:using-git-worktrees` context line (the orchestrator already works in its own worktree).
- Plan header: the "For agentic workers" line naming `subagent-driven-development` / `executing-plans` becomes a
  line saying each task is one breakdown step run by one specialist.
- Self-review: dropped "not a subagent dispatch".
- "Execution Handoff" is replaced by "From Plan to Breakdown": commit the plan on the task branch, then turn its
  tasks into `propose_breakdown` steps (title, specialist, brief, depends_on); the breakdown approval is the plan
  review.

`systematic-debugging/SKILL.md`
- `superpowers:test-driven-development` and `superpowers:verification-before-completion` become the plain skill
  names, as the skills are installed without the plugin prefix.

`test-driven-development/writing-good-tests.md`
- Dropped the `superpowers:writing-skills` reference (not shipped).

`test-driven-development/SKILL.md`, the systematic-debugging reference files, `verification-before-completion`
and `receiving-code-review`: unchanged.
