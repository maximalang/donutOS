import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { LuCookie, LuDownload, LuRocket } from "react-icons/lu";
import { SettingsPanel } from "@/components/ui/settings-panel";
import { cn } from "@/lib/utils";

/**
 * The local product's activity feed: what this desktop itself did — launches,
 * cookie-bot runs, engine installs. The vendor product showed the cloud's
 * "team activity" here; the local build has no cloud, so the same surface
 * reads the app's own journal (`local_activity.jsonl`).
 */

interface LocalActivityEntry {
  at: string;
  kind: string;
  subject: string;
  details?: string | null;
}

const KIND_ICONS: Record<string, typeof LuRocket> = {
  "profile.launched": LuRocket,
  "cookiebot.run": LuCookie,
  "engine.installed": LuDownload,
};

function kindLabelKey(kind: string): string {
  return `localActivity.kinds.${kind.replace(/\./g, "_")}`;
}

function formatTime(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString();
}

export function LocalActivityLog() {
  const { t } = useTranslation();
  const [entries, setEntries] = React.useState<LocalActivityEntry[]>([]);
  const [error, setError] = React.useState<string | null>(null);
  const [loading, setLoading] = React.useState(true);

  React.useEffect(() => {
    let cancelled = false;
    invoke<LocalActivityEntry[]>("list_local_activity", {
      kindPrefix: null,
      limit: 100,
    })
      .then((page) => {
        if (!cancelled) setEntries(page);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <SettingsPanel title={t("localActivity.title")}>
      <p className="text-xs text-muted-foreground">
        {t("localActivity.description")}
      </p>
      {loading ? (
        <p className="pt-2 text-xs text-muted-foreground">
          {t("localActivity.loading")}
        </p>
      ) : error ? (
        <p className="pt-2 text-xs text-destructive-text">{error}</p>
      ) : entries.length === 0 ? (
        <p className="pt-2 text-xs text-muted-foreground">
          {t("localActivity.empty")}
        </p>
      ) : (
        <ul className="flex max-h-80 flex-col gap-1 overflow-y-auto pt-2">
          {entries.map((entry, index) => {
            const Icon = KIND_ICONS[entry.kind] ?? LuRocket;
            return (
              <li
                key={`${entry.at}-${index}`}
                className={cn(
                  "flex items-center gap-3 rounded-lg px-2 py-1.5 text-xs",
                  "bg-foreground/4",
                )}
              >
                <Icon className="size-4 shrink-0 text-muted-foreground" />
                <span className="shrink-0 text-muted-foreground">
                  {formatTime(entry.at)}
                </span>
                <span className="min-w-0 flex-1 truncate font-medium">
                  {t(kindLabelKey(entry.kind), {
                    defaultValue: entry.kind,
                    subject: entry.subject,
                  })}
                </span>
                {entry.details && (
                  <span className="hidden shrink-0 text-muted-foreground @md:inline">
                    {entry.details}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </SettingsPanel>
  );
}
