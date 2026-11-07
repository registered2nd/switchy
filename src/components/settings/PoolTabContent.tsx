import { useState } from "react";
import { ShieldAlert } from "lucide-react";
import { motion } from "framer-motion";
import { useTranslation } from "react-i18next";
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ProxyPanel } from "@/components/proxy";
import { AutoFailoverConfigPanel } from "@/components/proxy/AutoFailoverConfigPanel";
import { FailoverQueueManager } from "@/components/proxy/FailoverQueueManager";
import { PoolControls } from "@/components/proxy/PoolControls";
import { RectifierConfigPanel } from "@/components/settings/RectifierConfigPanel";
import { GlobalProxySettings } from "@/components/settings/GlobalProxySettings";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ToggleRow } from "@/components/ui/toggle-row";
import { useProxyStatus } from "@/hooks/useProxyStatus";
import type { SettingsFormState } from "@/hooks/useSettings";

interface PoolTabContentProps {
  settings: SettingsFormState;
  onAutoSave: (updates: Partial<SettingsFormState>) => Promise<void>;
}

export function PoolTabContent({ settings, onAutoSave }: PoolTabContentProps) {
  const { t } = useTranslation();
  const [showProxyConfirm, setShowProxyConfirm] = useState(false);
  const [showFailoverConfirm, setShowFailoverConfirm] = useState(false);

  const {
    isRunning,
    startProxyServer,
    stopWithRestore,
    isPending: isProxyPending,
  } = useProxyStatus();

  const handleToggleProxy = async (checked: boolean) => {
    try {
      if (!checked) {
        await stopWithRestore();
      } else if (!settings?.proxyConfirmed) {
        setShowProxyConfirm(true);
      } else {
        await startProxyServer();
      }
    } catch (error) {
      console.error("Toggle proxy failed:", error);
    }
  };

  const handleProxyConfirm = async () => {
    setShowProxyConfirm(false);
    try {
      await onAutoSave({ proxyConfirmed: true });
      await startProxyServer();
    } catch (error) {
      console.error("Proxy confirm failed:", error);
    }
  };

  const handleFailoverToggleChange = (checked: boolean) => {
    if (checked && !settings?.failoverConfirmed) {
      setShowFailoverConfirm(true);
    } else {
      void onAutoSave({ enableFailoverToggle: checked });
    }
  };

  const handleFailoverConfirm = async () => {
    setShowFailoverConfirm(false);
    try {
      await onAutoSave({ failoverConfirmed: true, enableFailoverToggle: true });
    } catch (error) {
      console.error("Failover confirm failed:", error);
    }
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3 }}
      className="space-y-4"
    >
      <div className="space-y-4">
        <div className="max-w-[62ch]">
          <h2 className="font-display text-[17px] font-semibold tracking-tight">
            {t("pool.title")}
          </h2>
          <p className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground">
            {t("pool.description")}
          </p>
        </div>
        <PoolControls
          onToggleProxy={handleToggleProxy}
          isProxyPending={isProxyPending}
        />
      </div>

      <Accordion type="multiple" defaultValue={[]} className="w-full border-t border-border">
        {/* Local Proxy */}
        <AccordionItem
          value="proxy"
          className="border-b border-border"
        >
          <AccordionTrigger className="py-4 hover:no-underline">
            <div className="flex items-center gap-3">
              <div className="text-left">
                <h3 className="text-[14.5px] font-medium">
                  {t("settings.advanced.proxy.title")}
                </h3>
                <p className="text-[12.5px] font-normal text-muted-foreground">
                  {t("settings.advanced.proxy.description")}
                </p>
              </div>
              <span
                className={`ml-auto mr-3 text-[12.5px] ${isRunning ? "text-success" : "text-muted-foreground"}`}
              >
                {isRunning
                  ? t("settings.advanced.proxy.running")
                  : t("settings.advanced.proxy.stopped")}
              </span>
            </div>
          </AccordionTrigger>
          <AccordionContent className="pb-6 pt-1">
            <ProxyPanel
              enableLocalProxy={settings?.enableLocalProxy ?? false}
              onEnableLocalProxyChange={(checked) =>
                onAutoSave({ enableLocalProxy: checked })
              }
            />
          </AccordionContent>
        </AccordionItem>

        {/* Auto Failover */}
        <AccordionItem
          value="failover"
          className="border-b border-border"
        >
          <AccordionTrigger className="py-4 hover:no-underline">
            <div className="flex items-center gap-3">
              <div className="text-left">
                <h3 className="text-[14.5px] font-medium">
                  {t("settings.advanced.failover.title")}
                </h3>
                <p className="text-[12.5px] font-normal text-muted-foreground">
                  {t("settings.advanced.failover.description")}
                </p>
              </div>
            </div>
          </AccordionTrigger>
          <AccordionContent className="pb-6 pt-1">
            <div className="space-y-6">
              <ToggleRow
                icon={<ShieldAlert className="h-4 w-4 text-orange-500" />}
                title={t("settings.advanced.proxy.enableFailoverToggle")}
                description={t(
                  "settings.advanced.proxy.enableFailoverToggleDescription",
                )}
                checked={settings?.enableFailoverToggle ?? false}
                onCheckedChange={handleFailoverToggleChange}
              />

              {!isRunning && (
                <div className="p-4 rounded-lg bg-yellow-500/10 border border-yellow-500/20">
                  <p className="text-sm text-yellow-600 dark:text-yellow-400">
                    {t("proxy.failover.proxyRequired", {
                      defaultValue: "Start the local proxy to set up switching",
                    })}
                  </p>
                </div>
              )}

              <Tabs defaultValue="claude" className="w-full">
                <TabsList className="grid w-full grid-cols-3">
                  <TabsTrigger value="claude">Claude</TabsTrigger>
                  <TabsTrigger value="codex">Codex</TabsTrigger>
                  <TabsTrigger value="gemini">Gemini</TabsTrigger>
                </TabsList>
                <TabsContent value="claude" className="mt-4 space-y-6">
                  <div className="space-y-4">
                    <div>
                      <h4 className="text-sm font-semibold">
                        {t("proxy.failoverQueue.title")}
                      </h4>
                      <p className="text-xs text-muted-foreground">
                        {t("proxy.failoverQueue.description")}
                      </p>
                    </div>
                    <FailoverQueueManager
                      appType="claude"
                      disabled={!isRunning}
                    />
                  </div>
                  <div className="border-t border-border/50 pt-6">
                    <AutoFailoverConfigPanel
                      appType="claude"
                      disabled={!isRunning}
                    />
                  </div>
                </TabsContent>
                <TabsContent value="codex" className="mt-4 space-y-6">
                  <div className="space-y-4">
                    <div>
                      <h4 className="text-sm font-semibold">
                        {t("proxy.failoverQueue.title")}
                      </h4>
                      <p className="text-xs text-muted-foreground">
                        {t("proxy.failoverQueue.description")}
                      </p>
                    </div>
                    <FailoverQueueManager
                      appType="codex"
                      disabled={!isRunning}
                    />
                  </div>
                  <div className="border-t border-border/50 pt-6">
                    <AutoFailoverConfigPanel
                      appType="codex"
                      disabled={!isRunning}
                    />
                  </div>
                </TabsContent>
                <TabsContent value="gemini" className="mt-4 space-y-6">
                  <div className="space-y-4">
                    <div>
                      <h4 className="text-sm font-semibold">
                        {t("proxy.failoverQueue.title")}
                      </h4>
                      <p className="text-xs text-muted-foreground">
                        {t("proxy.failoverQueue.description")}
                      </p>
                    </div>
                    <FailoverQueueManager
                      appType="gemini"
                      disabled={!isRunning}
                    />
                  </div>
                  <div className="border-t border-border/50 pt-6">
                    <AutoFailoverConfigPanel
                      appType="gemini"
                      disabled={!isRunning}
                    />
                  </div>
                </TabsContent>
              </Tabs>
            </div>
          </AccordionContent>
        </AccordionItem>

        {/* Rectifier */}
        <AccordionItem
          value="rectifier"
          className="border-b border-border"
        >
          <AccordionTrigger className="py-4 hover:no-underline">
            <div className="flex items-center gap-3">
              <div className="text-left">
                <h3 className="text-[14.5px] font-medium">
                  {t("settings.advanced.rectifier.title")}
                </h3>
                <p className="text-[12.5px] font-normal text-muted-foreground">
                  {t("settings.advanced.rectifier.description")}
                </p>
              </div>
            </div>
          </AccordionTrigger>
          <AccordionContent className="pb-6 pt-1">
            <RectifierConfigPanel />
          </AccordionContent>
        </AccordionItem>

        {/* Global Outbound Proxy */}
        <AccordionItem
          value="globalProxy"
          className="border-b border-border"
        >
          <AccordionTrigger className="py-4 hover:no-underline">
            <div className="flex items-center gap-3">
              <div className="text-left">
                <h3 className="text-[14.5px] font-medium">
                  {t("settings.advanced.globalProxy.title")}
                </h3>
                <p className="text-[12.5px] font-normal text-muted-foreground">
                  {t("settings.advanced.globalProxy.description")}
                </p>
              </div>
            </div>
          </AccordionTrigger>
          <AccordionContent className="pb-6 pt-1">
            <GlobalProxySettings />
          </AccordionContent>
        </AccordionItem>
      </Accordion>

      <ConfirmDialog
        isOpen={showProxyConfirm}
        variant="info"
        title={t("confirm.proxy.title")}
        message={t("confirm.proxy.message")}
        confirmText={t("confirm.proxy.confirm")}
        onConfirm={() => void handleProxyConfirm()}
        onCancel={() => setShowProxyConfirm(false)}
      />

      <ConfirmDialog
        isOpen={showFailoverConfirm}
        variant="info"
        title={t("confirm.failover.title")}
        message={t("confirm.failover.message")}
        confirmText={t("confirm.failover.confirm")}
        onConfirm={() => void handleFailoverConfirm()}
        onCancel={() => setShowFailoverConfirm(false)}
      />
    </motion.div>
  );
}
