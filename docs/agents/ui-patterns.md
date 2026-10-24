# UI patterns

> Moved from AGENTS.md (deep dive; read when the task touches this area).

## Sub-page Dialog mode

A `<Dialog>` becomes a first-class app sub-page (no modal overlay, no center positioning) when `subPage` is passed. Pages like Account, Settings, Proxy Management, and Extension Management use this. The pattern for a sub-page with tabs:

```tsx
<Dialog open={isOpen} onOpenChange={onClose} subPage={subPage}>
  <DialogContent className="max-w-2xl flex flex-col">
    <Tabs defaultValue="account">
      <TabsList
        className={cn(
          "w-full",
          subPage &&
            "!bg-transparent !p-0 !h-auto !rounded-none justify-start gap-4",
        )}
      >
        <TabsTrigger
          value="account"
          className={cn(
            "flex-1",
            subPage &&
              "!flex-none !rounded-none !bg-transparent !shadow-none data-[state=active]:!bg-transparent data-[state=active]:!text-foreground data-[state=active]:!shadow-none text-muted-foreground hover:text-foreground !px-1 !py-1 text-xs",
          )}
        >
          Account
        </TabsTrigger>
        ...
      </TabsList>
      <TabsContent value="account" className="mt-4">...</TabsContent>
    </Tabs>
  </DialogContent>
</Dialog>
```

Reference implementations: `src/components/account-page.tsx`, `src/components/proxy-management-dialog.tsx`. Reuse the exact class strings; the overrides are tuned to match the rest of the sub-page chrome.

### Cross-component tab control

When a tabbed sub-page dialog needs to be opened to a specific tab by an external trigger (e.g. a keyboard shortcut that toggles `proxies` ↔ `vpns`), expose an `initialTab` prop and key the `Tabs` component off it. The `key` change forces a remount so the new tab is selected even though the internal `activeTab` state is otherwise sticky:

```tsx
<AnimatedTabs key={initialTab} defaultValue={initialTab} ...>
```

Reference implementations: `proxy-management-dialog.tsx`, `extension-management-dialog.tsx`, `integrations-dialog.tsx`. The owning page in `src/app/page.tsx` keeps one piece of `useState` per dialog (`proxyManagementInitialTab`, `extensionManagementInitialTab`, `integrationsInitialTab`) and flips it on repeated shortcut presses.

## Timelines (`OperationFlow`)

`src/components/ui/operation-flow.tsx` draws any measured operation as a row
of stations: settled stations wear a check, the current one is a ring (a
cross when `failed`), later ones wait as dots, wires fill as stations settle,
and `busy` sends a pulse along the wire into the station being worked on.
Pass `active` as the station the operation is AT, and `failed` when it
stopped there: a proxy check that cannot connect is `active={1}` (the proxy),
not the device. Reaching the last station with nothing failed settles the row.

## Keyboard shortcuts

All app-wide shortcuts live in `src/lib/shortcuts.ts`:

- `SHORTCUTS[]`: one entry per shortcut (id, label translation key, group, key, modifier flags). The label key must exist in every locale.
- `formatShortcut(s)` returns platform-correct token strings (`["⌘", "K"]` on mac, `["Ctrl", "K"]` elsewhere), used by both the shortcuts page and the command palette.
- `matchesShortcut(s, event)` matches a real `KeyboardEvent` and rejects the wrong-platform modifier so Ctrl+K on macOS never fires a `mod: true` shortcut.
- `matchesGroupDigit(event)` returns 1-9 if Mod+digit was pressed. Group switching is dynamic (driven by `orderedGroupTargets` in `page.tsx`) and isn't in the `SHORTCUTS` table.

Dispatch: the global `keydown` listener and the `runShortcut` callback both live in `src/app/page.tsx`. To add a new static shortcut:

1. Append to `SHORTCUTS` in `src/lib/shortcuts.ts`. Add the `ShortcutId` variant.
2. Add a `case "yourId":` in `runShortcut` in `page.tsx`.
3. Add the icon mapping in `src/components/command-palette.tsx::ICONS`.
4. Add `shortcuts.yourId` (label) to every locale file in `src/i18n/locales/`.

The command palette (Mod+K) is built on the shadcn `Command` primitive with a token-AND fuzzy filter (`fuzzyFilter` in `command-palette.tsx`). The `CommandDialog` wrapper now forwards `filter`/`shouldFilter` to the inner `Command` for callers that need custom matching.

### Semantic color classes (UI Theming)

- Available semantic color classes:
  - `background`, `foreground`: page/container background and text
  - `card`, `card-foreground`: card surfaces
  - `popover`, `popover-foreground`: dropdown/popover surfaces
  - `primary`, `primary-foreground`: primary actions
  - `secondary`, `secondary-foreground`: secondary actions
  - `muted`, `muted-foreground`: muted/disabled elements
  - `accent`, `accent-foreground`: accent highlights
  - `destructive`, `destructive-foreground`: errors, danger, delete actions
  - `success`, `success-foreground`: success states, valid indicators
  - `warning`, `warning-foreground`: warnings, caution messages
  - `border`: borders
  - `chart-1` through `chart-5`: data visualization
