import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { AppId } from "@/lib/api";
import type { ProviderFormValues } from "@/components/providers/forms/ProviderForm";

/** A Cursor provider holds `auth.json` and its `authInfo`; an Antigravity one the credential entry. */
const EMPTY_LOGIN: Record<string, Record<string, null>> = {
  cursor: { auth: null, authInfo: null },
  antigravity: { credential: null },
};

interface LoginProviderFormProps {
  appId: AppId;
  /** Editing keeps the stored login; only the name changes. */
  initialData?: { name?: string; settingsConfig?: Record<string, unknown> };
  onSubmit: (values: ProviderFormValues) => Promise<void> | void;
  onSubmittingChange?: (isSubmitting: boolean) => void;
}

/**
 * The provider form of an app whose provider is only a subscription login
 * (Cursor CLI, Antigravity CLI): a name. The login is filed into the card
 * by signing in while it is current.
 */
export function LoginProviderForm({
  appId,
  initialData,
  onSubmit,
  onSubmittingChange,
}: LoginProviderFormProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(initialData?.name ?? t(`apps.${appId}`));

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if (!name.trim()) {
      toast.error(
        t("providerForm.fillParameter", { label: t("loginProvider.name") }),
      );
      return;
    }
    onSubmittingChange?.(true);
    try {
      await onSubmit({
        name: name.trim(),
        websiteUrl: "",
        settingsConfig: JSON.stringify(
          initialData?.settingsConfig ?? EMPTY_LOGIN[appId] ?? {},
        ),
        presetCategory: "official",
      });
    } finally {
      onSubmittingChange?.(false);
    }
  };

  return (
    <form id="provider-form" onSubmit={handleSubmit} className="space-y-4">
      <div className="space-y-2">
        <Label htmlFor="login-provider-name">{t("loginProvider.name")}</Label>
        <Input
          id="login-provider-name"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
      </div>
      <p className="text-sm text-muted-foreground">
        {initialData
          ? t("loginProvider.editHint")
          : t(`loginProvider.hint.${appId}`)}
      </p>
    </form>
  );
}
