import { describe, expect, it } from "vitest";
import { providerPresets } from "@/config/claudeProviderPresets";

describe("AWS Bedrock Provider Presets", () => {
  const bedrockAksk = providerPresets.find(
    (p) => p.name === "AWS Bedrock (AKSK)",
  );

  it("should include AWS Bedrock (AKSK) preset", () => {
    expect(bedrockAksk).toBeDefined();
  });

  it("AKSK preset should have required AWS env variables", () => {
    const env = (bedrockAksk!.settingsConfig as any).env;
    expect(env).toHaveProperty("AWS_ACCESS_KEY_ID");
    expect(env).toHaveProperty("AWS_SECRET_ACCESS_KEY");
    expect(env).toHaveProperty("AWS_REGION");
    expect(env).toHaveProperty("CLAUDE_CODE_USE_BEDROCK", "1");
  });

  it("AKSK preset should have template values for AWS credentials", () => {
    expect(bedrockAksk!.templateValues).toBeDefined();
    expect(bedrockAksk!.templateValues!.AWS_ACCESS_KEY_ID).toBeDefined();
    expect(bedrockAksk!.templateValues!.AWS_SECRET_ACCESS_KEY).toBeDefined();
    expect(bedrockAksk!.templateValues!.AWS_REGION).toBeDefined();
    expect(bedrockAksk!.templateValues!.AWS_REGION.editorValue).toBe(
      "us-west-2",
    );
  });

  it("AKSK preset should have correct base URL template", () => {
    const env = (bedrockAksk!.settingsConfig as any).env;
    expect(env.ANTHROPIC_BASE_URL).toContain("bedrock-runtime");
    expect(env.ANTHROPIC_BASE_URL).toContain("${AWS_REGION}");
  });

  it("AKSK preset should have cloud_provider category", () => {
    expect(bedrockAksk!.category).toBe("cloud_provider");
  });

  it("AKSK preset should have Bedrock model as default", () => {
    const env = (bedrockAksk!.settingsConfig as any).env;
    expect(env.ANTHROPIC_MODEL).toContain("anthropic.claude");
  });

  const bedrockApiKey = providerPresets.find(
    (p) => p.name === "AWS Bedrock (API Key)",
  );

  it("should include AWS Bedrock (API Key) preset", () => {
    expect(bedrockApiKey).toBeDefined();
  });

  it("API Key preset should have apiKey field and AWS env variables", () => {
    const config = bedrockApiKey!.settingsConfig as any;
    expect(config).toHaveProperty("apiKey", "");
    expect(config.env).toHaveProperty("AWS_REGION");
    expect(config.env).toHaveProperty("CLAUDE_CODE_USE_BEDROCK", "1");
  });

  it("API Key preset should NOT have AKSK env variables", () => {
    const env = (bedrockApiKey!.settingsConfig as any).env;
    expect(env).not.toHaveProperty("AWS_ACCESS_KEY_ID");
    expect(env).not.toHaveProperty("AWS_SECRET_ACCESS_KEY");
  });

  it("API Key preset should have template values for region only", () => {
    expect(bedrockApiKey!.templateValues).toBeDefined();
    expect(bedrockApiKey!.templateValues!.AWS_REGION).toBeDefined();
    expect(bedrockApiKey!.templateValues!.AWS_REGION.editorValue).toBe(
      "us-west-2",
    );
  });

  it("API Key preset should have cloud_provider category", () => {
    expect(bedrockApiKey!.category).toBe("cloud_provider");
  });
});

describe("Coding plan presets", () => {
  const plans: Array<[string, string, string | undefined]> = [
    ["Z.ai GLM Coding Plan", "https://api.z.ai/api/anthropic", "zhipu"],
    ["MiniMax M Plan", "https://api.minimax.io/anthropic", "minimax"],
    ["Kimi Code Plan", "https://api.kimi.ai/coding/", "kimi"],
    ["Command Code", "https://api.commandcode.ai/provider", "commandcode"],
    ["OpenCode Go", "https://opencode.ai/inference/go/anthropic", undefined],
    [
      "Alibaba Cloud Token Plan",
      "https://token-plan.ap-southeast-1.maas.aliyuncs.com/apps/anthropic",
      undefined,
    ],
    [
      "Alibaba Cloud Coding Plan",
      "https://coding-intl.dashscope.aliyuncs.com/apps/anthropic",
      undefined,
    ],
    [
      "Tencent Cloud Token Plan",
      "https://tokenhub-intl.tencentcloudmaas.com/plan/anthropic",
      undefined,
    ],
    [
      "BytePlus ModelArk Coding Plan",
      "https://ark.ap-southeast.bytepluses.com/api/coding",
      undefined,
    ],
    ["DeepSeek", "https://api.deepseek.com/anthropic", undefined],
  ];

  it.each(plans)(
    "%s uses its international endpoint",
    (name, baseUrl, tokenPlan) => {
      const preset = providerPresets.find((p) => p.name === name);
      expect(preset).toBeDefined();
      const env = (preset!.settingsConfig as any).env;
      expect(env.ANTHROPIC_BASE_URL).toBe(baseUrl);
      expect(preset!.tokenPlan).toBe(tokenPlan);
    },
  );

  it("Kimi Code Plan takes its key in ANTHROPIC_API_KEY", () => {
    const kimi = providerPresets.find((p) => p.name === "Kimi Code Plan")!;
    expect(kimi.apiKeyField).toBe("ANTHROPIC_API_KEY");
    expect((kimi.settingsConfig as any).env).toHaveProperty(
      "ANTHROPIC_API_KEY",
      "",
    );
  });
});
