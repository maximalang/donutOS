"use client";

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { GoPlus } from "react-icons/go";
import { LuSparkles } from "react-icons/lu";
import { RecipeRecorder } from "@/components/agent-recipe-recorder";
import {
  emptyStep,
  isStepComplete,
  RecipeStepsEditor,
  tidyStep,
} from "@/components/agent-recipe-steps";
import { DeleteConfirmationDialog } from "@/components/delete-confirmation-dialog";
import { Button } from "@/components/ui/button";
import { FadingScrollArea } from "@/components/ui/fading-scroll-area";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { RippleButton } from "@/components/ui/ripple";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { translateBackendError } from "@/lib/backend-errors";
import { isFreeBuildSync } from "@/lib/free-build";
import {
  type AgentRecipe,
  createAgentRecipe,
  deleteAgentRecipe,
  generateRecipeFromText,
  getAgentRecipes,
  type RecipeStep,
  updateAgentRecipe,
} from "@/lib/recipes";
import { showErrorToast, showSuccessToast } from "@/lib/toast-utils";
import type { BrowserProfile } from "@/types";

interface AgentRecipesProps {
  /** Running profiles are the ones a recording can be made from. */
  profiles: BrowserProfile[];
}

export function AgentRecipes({ profiles }: AgentRecipesProps) {
  const { t } = useTranslation();
  const [recipes, setRecipes] = useState<AgentRecipe[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<unknown>(null);

  const loadRecipes = useCallback(async () => {
    setIsLoading(true);
    try {
      setRecipes(await getAgentRecipes());
      setError(null);
    } catch (loadError) {
      setError(loadError);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadRecipes();
  }, [loadRecipes]);
  /** The recipe being edited, `"new"` while composing one, or null. */
  const [editing, setEditing] = useState<AgentRecipe | "new" | null>(null);
  /** Steps a recording just produced, waiting to be reviewed and named. */
  const [recorded, setRecorded] = useState<RecipeStep[] | null>(null);
  const [pendingRemoval, setPendingRemoval] = useState<AgentRecipe | null>(
    null,
  );
  /** The AI draft box, free build only. */
  const [aiOpen, setAiOpen] = useState(false);
  const [aiText, setAiText] = useState("");
  const [aiBusy, setAiBusy] = useState(false);
  const [isRemoving, setIsRemoving] = useState(false);

  const confirmRemoval = useCallback(async () => {
    if (!pendingRemoval) return;
    setIsRemoving(true);
    try {
      await deleteAgentRecipe(pendingRemoval.id);
      showSuccessToast(t("agent.recipes.deleted"));
      setPendingRemoval(null);
      void loadRecipes();
    } catch (removeError) {
      showErrorToast(translateBackendError(t, removeError));
    } finally {
      setIsRemoving(false);
    }
  }, [pendingRemoval, loadRecipes, t]);

  return (
    <div
      data-slot="agent-recipes"
      className="flex min-h-0 flex-1 flex-col gap-3"
    >
      <div className="flex shrink-0 items-center justify-between gap-2">
        <p className="text-xs text-muted-foreground">
          {t("agent.recipes.hint")}
        </p>
        <RippleButton
          size="sm"
          className="flex h-8 items-center gap-1.5 rounded-lg text-xs"
          disabled={editing !== null}
          onClick={() => {
            setEditing("new");
          }}
        >
          <GoPlus className="size-3.5" />
          {t("agent.recipes.new")}
        </RippleButton>
        {isFreeBuildSync() && (
          <Button
            size="sm"
            variant="outline"
            className="h-8 items-center gap-1.5 rounded-lg text-xs"
            disabled={editing !== null || aiBusy}
            onClick={() => setAiOpen((open) => !open)}
          >
            <LuSparkles className="size-3.5" />
            {t("agent.recipes.aiGenerate")}
          </Button>
        )}
      </div>

      {isFreeBuildSync() && aiOpen && (
        <div className="flex shrink-0 flex-col gap-2 rounded-xl border border-border/60 bg-card p-3">
          <Textarea
            value={aiText}
            onChange={(event) => setAiText(event.target.value)}
            placeholder={t("agent.recipes.aiPlaceholder")}
            rows={3}
            className="resize-none text-sm"
          />
          <div className="flex items-center justify-between gap-2">
            <p className="text-[11px] text-muted-foreground">
              {t("agent.recipes.aiHint")}
            </p>
            <Button
              size="sm"
              className="h-8 items-center gap-1.5 rounded-lg text-xs"
              disabled={aiBusy || aiText.trim().length === 0}
              onClick={async () => {
                setAiBusy(true);
                try {
                  await generateRecipeFromText(aiText.trim());
                  showSuccessToast(t("agent.recipes.aiGenerated"));
                  setAiText("");
                  setAiOpen(false);
                  void loadRecipes();
                } catch (generateError) {
                  showErrorToast(translateBackendError(t, generateError));
                } finally {
                  setAiBusy(false);
                }
              }}
            >
              <LuSparkles className="size-3.5" />
              {aiBusy
                ? t("agent.recipes.aiGenerating")
                : t("agent.recipes.aiGenerate")}
            </Button>
          </div>
        </div>
      )}

      {error !== null && (
        <p className="shrink-0 rounded-xl bg-destructive/10 p-3 text-sm text-destructive-text">
          {translateBackendError(t, error)}
        </p>
      )}

      <RecipeRecorder
        profiles={profiles}
        onRecorded={(steps) => {
          // A recording is a draft, not a saved recipe: it opens the editor so
          // the person names it and sees every step before anything is stored.
          setRecorded(steps);
          setEditing("new");
        }}
      />

      {editing !== null && (
        <RecipeEditor
          recipe={editing === "new" ? null : editing}
          initialSteps={recorded}
          onCancel={() => {
            setEditing(null);
          }}
          onSaved={() => {
            setRecorded(null);
            setEditing(null);
            void loadRecipes();
          }}
        />
      )}

      <FadingScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-1 pr-1">
          {isLoading && recipes.length === 0 ? (
            Array.from({ length: 3 }, (_, index) => (
              <Skeleton
                key={`agent-recipe-skeleton-${index}`}
                className="h-12 w-full"
              />
            ))
          ) : recipes.length === 0 ? (
            <p
              data-slot="agent-recipes-empty"
              className="py-16 text-center text-sm text-muted-foreground"
            >
              {t("agent.recipes.empty")}
            </p>
          ) : (
            recipes.map((recipe) => (
              <div
                key={recipe.id}
                className="flex items-center gap-3 rounded-xl bg-foreground/3 p-3 transition-colors duration-150 hover:bg-foreground/5"
              >
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span className="truncate text-sm font-medium text-foreground">
                    {recipe.name}
                  </span>
                  <span className="truncate text-xs text-muted-foreground">
                    {t("agent.recipes.stepCount", {
                      steps: recipe.steps.length,
                    })}
                  </span>
                </div>
                <Button
                  variant="soft"
                  size="sm"
                  className="h-8 rounded-lg text-xs"
                  onClick={() => {
                    setEditing(recipe);
                  }}
                >
                  {t("common.buttons.edit")}
                </Button>
                <Button
                  variant="subtle"
                  size="sm"
                  className="h-8 rounded-lg text-xs hover:bg-destructive/10 hover:text-destructive-text"
                  onClick={() => {
                    setPendingRemoval(recipe);
                  }}
                >
                  {t("common.buttons.delete")}
                </Button>
              </div>
            ))
          )}
        </div>
      </FadingScrollArea>

      <DeleteConfirmationDialog
        isOpen={pendingRemoval !== null}
        onClose={() => {
          setPendingRemoval(null);
        }}
        onConfirm={() => {
          void confirmRemoval();
        }}
        title={t("agent.recipes.deleteTitle", {
          name: pendingRemoval?.name ?? "",
        })}
        description={t("agent.recipes.deleteDescription")}
        confirmButtonText={t("common.buttons.delete")}
        isLoading={isRemoving}
      />
    </div>
  );
}

function RecipeEditor({
  recipe,
  initialSteps,
  onCancel,
  onSaved,
}: {
  recipe: AgentRecipe | null;
  /** Steps a recording produced, for a new recipe that starts from one. */
  initialSteps?: RecipeStep[] | null;
  onCancel: () => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(recipe?.name ?? "");
  const [steps, setSteps] = useState<RecipeStep[]>(
    recipe?.steps ?? initialSteps ?? [],
  );
  const [isSaving, setIsSaving] = useState(false);

  // Editing a different recipe reuses this component, so the fields follow the
  // row the user actually clicked rather than keeping the previous one's text.
  useEffect(() => {
    setName(recipe?.name ?? "");
    setSteps(recipe?.steps ?? initialSteps ?? []);
  }, [recipe, initialSteps]);

  const canSave =
    name.trim().length > 0 &&
    steps.length > 0 &&
    steps.every(isStepComplete) &&
    !isSaving;

  const handleSave = useCallback(async () => {
    if (!canSave) return;
    setIsSaving(true);
    try {
      const payload = steps.map(tidyStep);
      if (recipe) {
        await updateAgentRecipe(recipe.id, name.trim(), payload);
        showSuccessToast(t("agent.recipes.updated"));
      } else {
        await createAgentRecipe(name.trim(), payload);
        showSuccessToast(t("agent.recipes.created"));
      }
      onSaved();
    } catch (saveError) {
      showErrorToast(translateBackendError(t, saveError));
    } finally {
      setIsSaving(false);
    }
  }, [canSave, recipe, name, steps, onSaved, t]);

  return (
    <form
      data-slot="agent-recipe-editor"
      className="flex shrink-0 flex-col gap-3 rounded-xl bg-foreground/3 p-3"
      onSubmit={(event) => {
        event.preventDefault();
        void handleSave();
      }}
    >
      <div className="flex flex-col gap-2">
        <Label
          htmlFor="agent-recipe-name"
          className="text-xs font-medium text-muted-foreground"
        >
          {t("agent.recipes.name")}
        </Label>
        <Input
          id="agent-recipe-name"
          value={name}
          placeholder={t("agent.recipes.namePlaceholder")}
          onChange={(event) => {
            setName(event.target.value);
          }}
        />
      </div>
      <div className="flex flex-col gap-2">
        <Label className="text-xs font-medium text-muted-foreground">
          {t("agent.recipes.steps")}
        </Label>
        <RecipeStepsEditor
          steps={steps}
          onChange={setSteps}
          disabled={isSaving}
        />
        <div className="flex items-center justify-between gap-2">
          <p className="text-xs text-muted-foreground">
            {t("agent.recipes.stepsHint")}
          </p>
          <Button
            variant="soft"
            size="sm"
            type="button"
            className="h-8 gap-1.5 rounded-lg text-xs"
            disabled={isSaving}
            onClick={() => {
              setSteps([...steps, emptyStep("navigate")]);
            }}
          >
            <GoPlus className="size-3.5" />
            {t("agent.recipes.addStep")}
          </Button>
        </div>
      </div>
      <div className="flex items-center justify-end gap-2">
        <Button
          variant="subtle"
          size="sm"
          className="h-8 rounded-lg text-xs"
          onClick={onCancel}
          type="button"
        >
          {t("common.buttons.cancel")}
        </Button>
        <RippleButton
          type="submit"
          size="sm"
          className="h-8 rounded-lg text-xs"
          disabled={!canSave}
        >
          {isSaving ? t("common.buttons.saving") : t("common.buttons.save")}
        </RippleButton>
      </div>
    </form>
  );
}
