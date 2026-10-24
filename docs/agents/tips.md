# Feature tips and the paid welcome

> Moved from AGENTS.md (deep dive; read when the task touches this area).

## Feature tips and the paid welcome

Tips are short feature walkthroughs: a looping SVG scene, a title, two or
three lines of copy, and a button into the feature. The catalog is
`src/lib/tips.ts` (ids, deep-link actions, the plan capability a tip needs,
the `inUse` relevance check); scenes live in `src/components/tips/scenes-*.tsx`
and are mapped in `scene-for.tsx`; the dialog is
`src/components/tips-dialog.tsx`; the flow (what to open when) is
`src/hooks/use-tips.ts`. State (`tips_auto_show`, `tips_seen`,
`tips_last_auto_shown_at`, `tips_next_auto_show_at`, `paid_welcome_seen_for`,
`cloud_plan_memory`) is in `AppSettings`, behind the `get_tips_state`,
`mark_tip_seen`, `set_tips_auto_show` and `observe_cloud_plan` commands.

- The automatic tip is one card (`mode: "single"`): no catalog, no pager, and
  its button is Done. The catalog with the pager opens only from the rail,
  the shortcut, the command palette and the paid welcome (`mode: "browse"`).
- It opens only after a settled launch (onboarding done, terms accepted,
  nothing modal open), never in the first-run session. When one opens, the
  backend draws the next time one may open, a random 2 to 5 days later
  (`next_auto_tip_at`), and stores it in `tips_next_auto_show_at`, so a
  restart keeps the same wait. The E2E harness seeds `tips_auto_show: false`;
  a test that wants the automatic tip passes `settings: { tips_auto_show: true }`
  or calls `set_tips_auto_show`.
- The automatic flow picks at random among the unseen tips whose feature is
  not in use (`pickAutoTip`). Each tip's `inUse` reads a `FeatureUsage`
  snapshot: profile fields, groups, extension groups, team, Cookie Bot
  enrolments and the sync server come from data `page.tsx` has loaded; API,
  remote control, fingerprint gate, proxy check history, default
  browser, trash, agent runs and sent feedback are read by `use-tips.ts`
  through existing commands, and only for the tips still in the running. An unknown fact
  never hides a tip. The command palette and import tips have no signal.
- Plan tips carry `requires`; they are listed only when the signed-in plan
  grants the capability. The paid welcome opens once per account when the
  backend sees it turn paid (free -> paid, or a paid account first seen right
  after signing in); `paid_welcome_due` in `settings_manager.rs` is the rule.
- Adding a tip: append to `TIPS` with an `inUse` check when the install
  shows whether the feature is used, write the scene, add
  `tips.items.<id>.{label,title,body,action}` to every locale, and run
  `pnpm test:tips`, which checks every locale carries every tip.
- Scenes are decorative and loop on their own clock (`useScene`); they show
  their resting frame under reduced motion and never hide the copy.
