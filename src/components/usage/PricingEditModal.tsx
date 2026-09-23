import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Save, Plus } from "lucide-react";
import { FullScreenPanel } from "@/components/common/FullScreenPanel";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useUpdateModelPricing } from "@/lib/query/usage";
import type { ModelPricing } from "@/types/usage";

interface PricingEditModalProps {
  open: boolean;
  model: ModelPricing;
  isNew?: boolean;
  onClose: () => void;
}

export function PricingEditModal({
  open,
  model,
  isNew = false,
  onClose,
}: PricingEditModalProps) {
  const { t } = useTranslation();
  const updatePricing = useUpdateModelPricing();

  const [formData, setFormData] = useState({
    modelId: model.modelId,
    displayName: model.displayName,
    inputCost: model.inputCostPerMillion,
    outputCost: model.outputCostPerMillion,
    cacheReadCost: model.cacheReadCostPerMillion,
    cacheCreationCost: model.cacheCreationCostPerMillion,
  });

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    // Validate the model ID
    if (isNew && !formData.modelId.trim()) {
      toast.error(t("usage.modelIdRequired", "Model ID is required"));
      return;
    }

    // Must be non-negative
    const values = [
      formData.inputCost,
      formData.outputCost,
      formData.cacheReadCost,
      formData.cacheCreationCost,
    ];

    for (const value of values) {
      const num = parseFloat(value);
      if (isNaN(num) || num < 0) {
        toast.error(t("usage.invalidPrice", "Price must be non-negative"));
        return;
      }
    }

    try {
      await updatePricing.mutateAsync({
        modelId: isNew ? formData.modelId : model.modelId,
        displayName: formData.displayName,
        inputCost: formData.inputCost,
        outputCost: formData.outputCost,
        cacheReadCost: formData.cacheReadCost,
        cacheCreationCost: formData.cacheCreationCost,
      });

      toast.success(
        isNew
          ? t("usage.pricingAdded", "Pricing added")
          : t("usage.pricingUpdated", "Pricing updated"),
        { closeButton: true },
      );

      onClose();
    } catch (error) {
      toast.error(String(error));
    }
  };

  return (
    <FullScreenPanel
      isOpen={open}
      title={
        isNew
          ? t("usage.addPricing", "Add Pricing")
          : `${t("usage.editPricing", "Edit Pricing")} - ${model.modelId}`
      }
      onClose={onClose}
      footer={
        <Button
          type="submit"
          form="pricing-form"
          disabled={updatePricing.isPending}
        >
          {isNew ? (
            <Plus className="h-4 w-4 mr-2" />
          ) : (
            <Save className="h-4 w-4 mr-2" />
          )}
          {updatePricing.isPending
            ? t("common.saving", "Saving...")
            : isNew
              ? t("common.add", "Add")
              : t("common.save", "Save")}
        </Button>
      }
    >
      <form id="pricing-form" onSubmit={handleSubmit} className="space-y-6">
        {isNew && (
          <div className="space-y-2">
            <Label htmlFor="modelId">{t("usage.modelId", "Model ID")}</Label>
            <Input
              id="modelId"
              value={formData.modelId}
              onChange={(e) =>
                setFormData({ ...formData, modelId: e.target.value })
              }
              placeholder={t("usage.modelIdPlaceholder", {
                defaultValue: "e.g., claude-3-5-sonnet-20241022",
              })}
              required
            />
          </div>
        )}

        <div className="space-y-2">
          <Label htmlFor="displayName">
            {t("usage.displayName", "Display Name")}
          </Label>
          <Input
            id="displayName"
            value={formData.displayName}
            onChange={(e) =>
              setFormData({ ...formData, displayName: e.target.value })
            }
            placeholder={t("usage.displayNamePlaceholder", {
              defaultValue: "e.g., Claude 3.5 Sonnet",
            })}
            required
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="inputCost">
            {t(
              "usage.inputCostPerMillion",
              "Input Cost (per million tokens, USD)",
            )}
          </Label>
          <Input
            id="inputCost"
            type="number"
            step="0.01"
            min="0"
            value={formData.inputCost}
            onChange={(e) =>
              setFormData({ ...formData, inputCost: e.target.value })
            }
            required
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="outputCost">
            {t(
              "usage.outputCostPerMillion",
              "Output Cost (per million tokens, USD)",
            )}
          </Label>
          <Input
            id="outputCost"
            type="number"
            step="0.01"
            min="0"
            value={formData.outputCost}
            onChange={(e) =>
              setFormData({ ...formData, outputCost: e.target.value })
            }
            required
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="cacheReadCost">
            {t(
              "usage.cacheReadCostPerMillion",
              "Cache Read Cost (per million tokens, USD)",
            )}
          </Label>
          <Input
            id="cacheReadCost"
            type="number"
            step="0.01"
            min="0"
            value={formData.cacheReadCost}
            onChange={(e) =>
              setFormData({ ...formData, cacheReadCost: e.target.value })
            }
            required
          />
        </div>

        <div className="space-y-2">
          <Label htmlFor="cacheCreationCost">
            {t(
              "usage.cacheCreationCostPerMillion",
              "Cache Write Cost (per million tokens, USD)",
            )}
          </Label>
          <Input
            id="cacheCreationCost"
            type="number"
            step="0.01"
            min="0"
            value={formData.cacheCreationCost}
            onChange={(e) =>
              setFormData({ ...formData, cacheCreationCost: e.target.value })
            }
            required
          />
        </div>
      </form>
    </FullScreenPanel>
  );
}
