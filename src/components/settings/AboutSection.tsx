import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { getVersion } from "@tauri-apps/api/app";
import { Badge } from "@/components/ui/badge";
import appIcon from "@/assets/icons/app-icon.png";

export function AboutSection() {
  const { t } = useTranslation();
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    getVersion()
      .then((value) => {
        if (active) setVersion(value);
      })
      .catch((error) => {
        console.error("[AboutSection] Failed to read the app version", error);
        if (active) setVersion("");
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <section className="space-y-4">

      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <img src={appIcon} alt="Switchy" className="h-5 w-5" />
          <h4 className="text-lg font-semibold text-foreground">Switchy</h4>
          <Badge variant="outline" className="gap-1.5 bg-background/80">
            <span className="text-muted-foreground">{t("common.version")}</span>
            {version === null ? (
              <Loader2 className="h-3 w-3 animate-spin" />
            ) : (
              <span className="font-medium">
                {version ? `v${version}` : t("common.unknown")}
              </span>
            )}
          </Badge>
        </div>
        <p className="text-sm text-muted-foreground">{t("app.description")}</p>
      </div>
    </section>
  );
}
