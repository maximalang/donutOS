import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { LuKeyRound, LuPlus, LuTrash2 } from "react-icons/lu";
import { LoadingButton } from "@/components/loading-button";
import { Button } from "@/components/ui/button";
import { CopyToClipboard } from "@/components/ui/copy-to-clipboard";
import { Input } from "@/components/ui/input";
import { SettingsPanel } from "@/components/ui/settings-panel";

/**
 * Named API members with roles: the local product's team seats. Each member
 * is an automation client of the local API — a fleet bot, an operator's
 * script — with its own token (shown once) and a role that gates what it may
 * call. The master token stays the implicit owner.
 */

type Role = "owner" | "operator" | "viewer";

interface ApiMember {
  id: string;
  name: string;
  role: Role;
  created_at: string;
  last_used_at: string | null;
}

interface IssuedMember {
  id: string;
  name: string;
  role: Role;
  token: string;
}

const ROLES: Role[] = ["operator", "viewer", "owner"];

function formatTime(iso: string | null): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString();
}

export function ApiMembersPanel() {
  const { t } = useTranslation();
  const [members, setMembers] = React.useState<ApiMember[]>([]);
  const [name, setName] = React.useState("");
  const [role, setRole] = React.useState<Role>("operator");
  const [issued, setIssued] = React.useState<IssuedMember | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);

  const reload = React.useCallback(() => {
    invoke<ApiMember[]>("list_api_members")
      .then(setMembers)
      .catch((e: unknown) => setError(String(e)));
  }, []);

  React.useEffect(() => {
    reload();
  }, [reload]);

  const handleCreate = async () => {
    setBusy(true);
    setError(null);
    try {
      const member = await invoke<IssuedMember>("create_api_member", {
        name,
        role,
      });
      setIssued(member);
      setName("");
      reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleRevoke = async (id: string) => {
    setBusy(true);
    setError(null);
    try {
      await invoke("revoke_api_member", { id });
      reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <SettingsPanel title={t("integrations.members.title")}>
      <p className="text-xs text-muted-foreground">
        {t("integrations.members.description")}
      </p>

      {error && <p className="text-xs text-destructive-text">{error}</p>}

      {issued && (
        <div className="flex flex-col gap-1.5 rounded-lg bg-foreground/4 p-3">
          <p className="flex items-center gap-2 text-xs font-medium">
            <LuKeyRound className="size-3.5" />
            {t("integrations.members.tokenOnce", { name: issued.name })}
          </p>
          <div className="flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded bg-background px-2 py-1 font-mono text-xs">
              {issued.token}
            </code>
            <CopyToClipboard
              text={issued.token}
              successMessage={t("integrations.tokenCopied")}
            />
          </div>
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={t("integrations.members.namePlaceholder")}
          className="h-8 w-40 text-xs"
          maxLength={40}
        />
        <select
          value={role}
          onChange={(e) => setRole(e.target.value as Role)}
          className="h-8 rounded-lg border border-border bg-background px-2 text-xs"
          aria-label={t("integrations.members.roleLabel")}
        >
          {ROLES.map((r) => (
            <option key={r} value={r}>
              {t(`integrations.members.roles.${r}`)}
            </option>
          ))}
        </select>
        <LoadingButton
          size="sm"
          variant="soft"
          className="h-8 gap-1.5 rounded-lg text-xs"
          isLoading={busy}
          disabled={name.trim().length === 0}
          onClick={() => void handleCreate()}
        >
          <LuPlus className="size-3.5" />
          {t("integrations.members.create")}
        </LoadingButton>
      </div>

      {members.length > 0 && (
        <ul className="flex flex-col gap-1">
          {members.map((member) => (
            <li
              key={member.id}
              className="flex items-center gap-3 rounded-lg bg-foreground/4 px-3 py-1.5 text-xs"
            >
              <span className="min-w-0 flex-1 truncate font-medium">
                {member.name}
              </span>
              <span className="shrink-0 text-muted-foreground">
                {t(`integrations.members.roles.${member.role}`)}
              </span>
              <span className="hidden shrink-0 text-muted-foreground @xl:inline">
                {t("integrations.members.lastUsed", {
                  time: formatTime(member.last_used_at),
                })}
              </span>
              <Button
                type="button"
                variant="subtle"
                size="icon"
                className="size-6 rounded-md text-destructive-text hover:bg-destructive/10"
                aria-label={t("integrations.members.revoke", {
                  name: member.name,
                })}
                disabled={busy}
                onClick={() => void handleRevoke(member.id)}
              >
                <LuTrash2 className="size-3.5" />
              </Button>
            </li>
          ))}
        </ul>
      )}
    </SettingsPanel>
  );
}
