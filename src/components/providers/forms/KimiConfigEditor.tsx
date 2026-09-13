import React, { useState } from "react";
import {
  KimiCredentialsSection,
  KimiConfigSection,
} from "./KimiConfigSections";
import { KimiCommonConfigModal } from "./KimiCommonConfigModal";

interface KimiConfigEditorProps {
  credentialsValue: string;

  configValue: string;

  onCredentialsChange: (value: string) => void;

  onConfigChange: (value: string) => void;

  onCredentialsBlur?: () => void;

  useCommonConfig: boolean;

  onCommonConfigToggle: (checked: boolean) => void;

  commonConfigSnippet: string;

  onCommonConfigSnippetChange: (value: string) => boolean;

  onCommonConfigErrorClear: () => void;

  commonConfigError: string;

  credentialsError: string;

  configError: string; // config.toml 错误提示

  onExtract?: () => void;

  isExtracting?: boolean;
}

const KimiConfigEditor: React.FC<KimiConfigEditorProps> = ({
  credentialsValue,
  configValue,
  onCredentialsChange,
  onConfigChange,
  onCredentialsBlur,
  useCommonConfig,
  onCommonConfigToggle,
  commonConfigSnippet,
  onCommonConfigSnippetChange,
  onCommonConfigErrorClear,
  commonConfigError,
  credentialsError,
  configError,
  onExtract,
  isExtracting,
}) => {
  const [isCommonConfigModalOpen, setIsCommonConfigModalOpen] = useState(false);

  const handleCloseCommonConfigModal = () => {
    onCommonConfigErrorClear();
    setIsCommonConfigModalOpen(false);
  };

  return (
    <div className="space-y-6">
      {/* Config TOML Section（供应商、模型、default_model 都在这里） */}
      <KimiConfigSection
        value={configValue}
        onChange={onConfigChange}
        useCommonConfig={useCommonConfig}
        onCommonConfigToggle={onCommonConfigToggle}
        onEditCommonConfig={() => setIsCommonConfigModalOpen(true)}
        commonConfigError={commonConfigError}
        configError={configError}
      />

      {/* Credentials JSON Section */}
      <KimiCredentialsSection
        value={credentialsValue}
        onChange={onCredentialsChange}
        onBlur={onCredentialsBlur}
        error={credentialsError}
      />

      {/* Common Config Modal */}
      <KimiCommonConfigModal
        isOpen={isCommonConfigModalOpen}
        onClose={handleCloseCommonConfigModal}
        value={commonConfigSnippet}
        onSave={onCommonConfigSnippetChange}
        error={commonConfigError}
        onExtract={onExtract}
        isExtracting={isExtracting}
      />
    </div>
  );
};

export default KimiConfigEditor;
