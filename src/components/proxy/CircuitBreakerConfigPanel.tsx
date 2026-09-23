import {
  useCircuitBreakerConfig,
  useUpdateCircuitBreakerConfig,
} from "@/lib/query/failover";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { useState, useEffect } from "react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";

/**
 * Circuit breaker settings panel
 * Lets the user tune circuit breaker parameters
 */
export function CircuitBreakerConfigPanel() {
  const { t } = useTranslation();
  const { data: config, isLoading } = useCircuitBreakerConfig();
  const updateConfig = useUpdateCircuitBreakerConfig();

  // String state so inputs can be cleared completely
  const [formData, setFormData] = useState({
    failureThreshold: "5",
    successThreshold: "2",
    timeoutSeconds: "60",
    errorRateThreshold: "50", // Stored as a percentage
    minRequests: "10",
  });

  // Fill the form once the config loads
  useEffect(() => {
    if (config) {
      setFormData({
        failureThreshold: String(config.failureThreshold),
        successThreshold: String(config.successThreshold),
        timeoutSeconds: String(config.timeoutSeconds),
        errorRateThreshold: String(Math.round(config.errorRateThreshold * 100)),
        minRequests: String(config.minRequests),
      });
    }
  }, [config]);

  const handleSave = async () => {
    // Parse a number; NaN means invalid input
    const parseNum = (val: string) => {
      const trimmed = val.trim();
      // Digits only
      if (!/^-?\d+$/.test(trimmed)) return NaN;
      return parseInt(trimmed);
    };

    // Valid range per field
    const ranges = {
      failureThreshold: { min: 1, max: 20 },
      successThreshold: { min: 1, max: 10 },
      timeoutSeconds: { min: 0, max: 300 },
      errorRateThreshold: { min: 0, max: 100 },
      minRequests: { min: 5, max: 100 },
    };

    // Parse the raw values
    const raw = {
      failureThreshold: parseNum(formData.failureThreshold),
      successThreshold: parseNum(formData.successThreshold),
      timeoutSeconds: parseNum(formData.timeoutSeconds),
      errorRateThreshold: parseNum(formData.errorRateThreshold),
      minRequests: parseNum(formData.minRequests),
    };

    // Range check (NaN counts as invalid)
    const errors: string[] = [];
    const checkRange = (
      value: number,
      range: { min: number; max: number },
      label: string,
    ) => {
      if (isNaN(value) || value < range.min || value > range.max) {
        errors.push(`${label}: ${range.min}-${range.max}`);
      }
    };

    checkRange(
      raw.failureThreshold,
      ranges.failureThreshold,
      t("circuitBreaker.failureThreshold", "Failure Threshold"),
    );
    checkRange(
      raw.successThreshold,
      ranges.successThreshold,
      t("circuitBreaker.successThreshold", "Success Threshold"),
    );
    checkRange(
      raw.timeoutSeconds,
      ranges.timeoutSeconds,
      t("circuitBreaker.timeoutSeconds", "Timeout (seconds)"),
    );
    checkRange(
      raw.errorRateThreshold,
      ranges.errorRateThreshold,
      t("circuitBreaker.errorRateThreshold", "Error Rate Threshold (%)"),
    );
    checkRange(
      raw.minRequests,
      ranges.minRequests,
      t("circuitBreaker.minRequests", "Minimum Requests"),
    );

    if (errors.length > 0) {
      toast.error(
        t("circuitBreaker.validationFailed", {
          fields: errors.join("; "),
          defaultValue: `The following fields are out of valid range: ${errors.join("; ")}`,
        }),
      );
      return;
    }

    try {
      await updateConfig.mutateAsync({
        failureThreshold: raw.failureThreshold,
        successThreshold: raw.successThreshold,
        timeoutSeconds: raw.timeoutSeconds,
        errorRateThreshold: raw.errorRateThreshold / 100,
        minRequests: raw.minRequests,
      });
      toast.success(
        t("circuitBreaker.configSaved", "Circuit breaker configuration saved"),
        {
          closeButton: true,
        },
      );
    } catch (error) {
      toast.error(
        t("circuitBreaker.saveFailed", "Save failed") + ": " + String(error),
      );
    }
  };

  const handleReset = () => {
    if (config) {
      setFormData({
        failureThreshold: String(config.failureThreshold),
        successThreshold: String(config.successThreshold),
        timeoutSeconds: String(config.timeoutSeconds),
        errorRateThreshold: String(Math.round(config.errorRateThreshold * 100)),
        minRequests: String(config.minRequests),
      });
    }
  };

  if (isLoading) {
    return (
      <div className="text-sm text-muted-foreground">
        {t("circuitBreaker.loading", "Loading...")}
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-lg font-semibold">
          {t("circuitBreaker.title", "Circuit Breaker Configuration")}
        </h3>
        <p className="text-sm text-muted-foreground mt-1">
          {t(
            "circuitBreaker.description",
            "Adjust circuit breaker parameters to control fault detection and recovery behavior",
          )}
        </p>
      </div>

      <div className="h-px bg-border my-4" />

      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        {/* Failure threshold */}
        <div className="space-y-2">
          <Label htmlFor="failureThreshold">
            {t("circuitBreaker.failureThreshold", "Failure Threshold")}
          </Label>
          <Input
            id="failureThreshold"
            type="number"
            min="1"
            max="20"
            value={formData.failureThreshold}
            onChange={(e) =>
              setFormData({ ...formData, failureThreshold: e.target.value })
            }
          />
          <p className="text-xs text-muted-foreground">
            {t(
              "circuitBreaker.failureThresholdHint",
              "How many consecutive failures trigger the circuit breaker",
            )}
          </p>
        </div>

        {/* Timeout */}
        <div className="space-y-2">
          <Label htmlFor="timeoutSeconds">
            {t("circuitBreaker.timeoutSeconds", "Timeout (seconds)")}
          </Label>
          <Input
            id="timeoutSeconds"
            type="number"
            min="0"
            max="300"
            value={formData.timeoutSeconds}
            onChange={(e) =>
              setFormData({ ...formData, timeoutSeconds: e.target.value })
            }
          />
          <p className="text-xs text-muted-foreground">
            {t(
              "circuitBreaker.timeoutSecondsHint",
              "How long to wait before attempting recovery (half-open state)",
            )}
          </p>
        </div>

        {/* Success threshold */}
        <div className="space-y-2">
          <Label htmlFor="successThreshold">
            {t("circuitBreaker.successThreshold", "Success Threshold")}
          </Label>
          <Input
            id="successThreshold"
            type="number"
            min="1"
            max="10"
            value={formData.successThreshold}
            onChange={(e) =>
              setFormData({ ...formData, successThreshold: e.target.value })
            }
          />
          <p className="text-xs text-muted-foreground">
            {t(
              "circuitBreaker.successThresholdHint",
              "How many successes in half-open state to close the circuit breaker",
            )}
          </p>
        </div>

        {/* Error rate threshold */}
        <div className="space-y-2">
          <Label htmlFor="errorRateThreshold">
            {t("circuitBreaker.errorRateThreshold", "Error Rate Threshold (%)")}
          </Label>
          <Input
            id="errorRateThreshold"
            type="number"
            min="0"
            max="100"
            step="5"
            value={formData.errorRateThreshold}
            onChange={(e) =>
              setFormData({ ...formData, errorRateThreshold: e.target.value })
            }
          />
          <p className="text-xs text-muted-foreground">
            {t(
              "circuitBreaker.errorRateThresholdHint",
              "Open circuit breaker when error rate exceeds this value",
            )}
          </p>
        </div>

        {/* Minimum requests */}
        <div className="space-y-2">
          <Label htmlFor="minRequests">
            {t("circuitBreaker.minRequests", "Minimum Requests")}
          </Label>
          <Input
            id="minRequests"
            type="number"
            min="5"
            max="100"
            value={formData.minRequests}
            onChange={(e) =>
              setFormData({ ...formData, minRequests: e.target.value })
            }
          />
          <p className="text-xs text-muted-foreground">
            {t(
              "circuitBreaker.minRequestsHint",
              "Minimum requests before calculating error rate",
            )}
          </p>
        </div>
      </div>

      <div className="flex gap-3">
        <Button onClick={handleSave} disabled={updateConfig.isPending}>
          {updateConfig.isPending
            ? t("common.saving", "Saving...")
            : t("circuitBreaker.saveConfig", "Save Configuration")}
        </Button>
        <Button
          variant="outline"
          onClick={handleReset}
          disabled={updateConfig.isPending}
        >
          {t("common.reset", "Reset")}
        </Button>
      </div>

      {/* Explanations */}
      <div className="p-4 bg-muted/50 rounded-lg space-y-2 text-sm">
        <h4 className="font-medium">
          {t("circuitBreaker.instructionsTitle", "Configuration Instructions")}
        </h4>
        <ul className="space-y-1 text-muted-foreground">
          <li>
            •{" "}
            <strong>
              {t("circuitBreaker.failureThreshold", "Failure Threshold")}
            </strong>
            {t("circuitBreaker.labelSeparator", ": ")}
            {t(
              "circuitBreaker.instructions.failureThreshold",
              "Circuit breaker opens when consecutive failures reach this count",
            )}
          </li>
          <li>
            •{" "}
            <strong>
              {t("circuitBreaker.timeoutSeconds", "Timeout (seconds)")}
            </strong>
            {t("circuitBreaker.labelSeparator", ": ")}
            {t(
              "circuitBreaker.instructions.timeout",
              "After circuit breaker opens, wait this time before attempting half-open",
            )}
          </li>
          <li>
            •{" "}
            <strong>
              {t("circuitBreaker.successThreshold", "Success Threshold")}
            </strong>
            {t("circuitBreaker.labelSeparator", ": ")}
            {t(
              "circuitBreaker.instructions.successThreshold",
              "In half-open state, close circuit breaker when successes reach this count",
            )}
          </li>
          <li>
            •{" "}
            <strong>
              {t(
                "circuitBreaker.errorRateThreshold",
                "Error Rate Threshold (%)",
              )}
            </strong>
            {t("circuitBreaker.labelSeparator", ": ")}
            {t(
              "circuitBreaker.instructions.errorRate",
              "Circuit breaker opens when error rate exceeds this value",
            )}
          </li>
          <li>
            •{" "}
            <strong>
              {t("circuitBreaker.minRequests", "Minimum Requests")}
            </strong>
            {t("circuitBreaker.labelSeparator", ": ")}
            {t(
              "circuitBreaker.instructions.minRequests",
              "Error rate is only calculated after request count reaches this value",
            )}
          </li>
        </ul>
      </div>
    </div>
  );
}
