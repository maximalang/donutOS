"use client";

// The AI provider panel: the person's own OpenAI-compatible endpoint that
// answers the agent conversation and drafts recipes. Everything here is
// fields and toggles; the key travels once, into its sealed file, and never
// comes back out.

import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LuBrainCircuit, LuSparkles } from "react-icons/lu";
import { AnimatedSwitch } from "@/components/ui/animated-switch";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  FeatureIcon,
  SECTION_LABEL_CLASS,
  SettingsPanel,
  SettingsRow,
} from "@/components/ui/settings-panel";

type AiStatus = {
  enabled: boolean;
  baseUrl?: string | null;
  model?: string | null;
  hasKey: boolean;
};

export function AiProviderPanel() {
  const { t } = useTranslation();
  const [enabled, setEnabled] = useState(false);
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [hasKey, setHasKey] = useState(false);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const status = await invoke<AiStatus>("get_ai_provider_status");
      setEnabled(status.enabled);
      setBaseUrl(status.baseUrl ?? "");
      setModel(status.model ?? "");
      setHasKey(status.hasKey);
    } catch (error) {
      setMessage(String(error));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const save = async () => {
    setSaving(true);
    setMessage(null);
    try {
      const status = await invoke<AiStatus>("save_ai_provider_config", {
        enabled,
        baseUrl: baseUrl || null,
        model: model || null,
        apiKey: apiKey || null,
      });
      setEnabled(status.enabled);
      setBaseUrl(status.baseUrl ?? "");
      setModel(status.model ?? "");
      setHasKey(status.hasKey);
      setApiKey("");
      setMessage(t("settings.ai.saved"));
    } catch (error) {
      setMessage(String(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <SettingsPanel>
      <div className="flex items-start gap-3">
        <FeatureIcon active={enabled}>
          <LuBrainCircuit />
        </FeatureIcon>
        <div className="min-w-0 flex-1">
          <p className={SECTION_LABEL_CLASS}>{t("settings.ai.title")}</p>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("settings.ai.description")}
          </p>
        </div>
      </div>

      <SettingsRow
        label={t("settings.ai.enable")}
        description={t("settings.ai.enableDescription")}
      >
        <AnimatedSwitch checked={enabled} onCheckedChange={setEnabled} />
      </SettingsRow>

      <div className="grid gap-4">
        <div className="grid gap-2">
          <Label htmlFor="ai-base-url">{t("settings.ai.baseUrl")}</Label>
          <Input
            id="ai-base-url"
            value={baseUrl}
            onChange={(event) => setBaseUrl(event.target.value)}
            placeholder={t("settings.ai.baseUrlPlaceholder")}
            autoComplete="off"
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="ai-model">{t("settings.ai.model")}</Label>
          <Input
            id="ai-model"
            value={model}
            onChange={(event) => setModel(event.target.value)}
            placeholder={t("settings.ai.modelPlaceholder")}
            autoComplete="off"
          />
        </div>
        <div className="grid gap-2">
          <Label htmlFor="ai-key">
            {t("settings.ai.apiKey")}
            {hasKey && (
              <span className="ml-2 text-xs font-normal text-muted-foreground">
                {t("settings.ai.keyStored")}
              </span>
            )}
          </Label>
          <Input
            id="ai-key"
            type="password"
            value={apiKey}
            onChange={(event) => setApiKey(event.target.value)}
            placeholder={t("settings.ai.apiKeyPlaceholder")}
            autoComplete="new-password"
          />
        </div>
      </div>

      <div className="flex items-center gap-3">
        <Button onClick={save} disabled={saving}>
          <LuSparkles className="size-4" aria-hidden="true" />
          {saving ? t("common.buttons.saving") : t("settings.ai.save")}
        </Button>
        {message && (
          <span className="text-sm text-muted-foreground">{message}</span>
        )}
      </div>

      <p className="text-xs text-muted-foreground">{t("settings.ai.hint")}</p>
    </SettingsPanel>
  );
}
