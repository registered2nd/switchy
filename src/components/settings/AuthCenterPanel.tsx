import { useTranslation } from "react-i18next";
import { CopilotAuthSection } from "@/components/providers/forms/CopilotAuthSection";
import { SettingsSection } from "@/components/settings/SettingsSection";

export function AuthCenterPanel() {
  const { t } = useTranslation();

  return (
    <SettingsSection
      title="GitHub Copilot"
      description={t("settings.authCenter.copilotDescription", {
        defaultValue:
          "Sign in with GitHub to use your Copilot subscription as the model behind Claude Code: add the GitHub Copilot provider under Claude after signing in.",
      })}
    >
      <CopilotAuthSection />
    </SettingsSection>
  );
}
