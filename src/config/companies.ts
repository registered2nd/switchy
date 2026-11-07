/**
 * The companies Switchy holds accounts with, in sidebar order.
 *
 * A company lists the sign-ins of its own programs (Claude in Claude Code,
 * ChatGPT in Codex, ...), the relay and gateway accounts that serve its
 * programs, its key-based subscription plans, and, where the company has a
 * balance call, an optional pay-as-you-go balance monitor. What a plan puts
 * into each program is built from this catalog and the programs' presets
 * into `planPrograms.json`, from which the backend makes an account's
 * providers (tests/config/planPrograms.test.ts keeps the two in step).
 */
import type { AppId } from "@/lib/api";
import type { Provider } from "@/types";
import { KIMI_OFFICIAL_CONFIG } from "@/config/kimiProviderPresets";
import {
  isKimiOfficialConfig,
  extractCodexBaseUrl,
  extractKimiBaseUrl,
} from "@/utils/providerConfigUtils";

export type CompanyId =
  | "anthropic"
  | "openai"
  | "google"
  | "kimi"
  | "cursor"
  | "zai"
  | "minimax"
  | "deepseek"
  | "opencode"
  | "commandcode"
  | "alibaba"
  | "tencent"
  | "byteplus"
  | "stepfun";

/** Cards no company claims: pay-as-you-go keys that run in a program. */
export const OTHER_COMPANY = "other";
export type SidebarEntryId = CompanyId | typeof OTHER_COMPANY;

export type RegionId = "intl" | "cn";

/** A plan's settings for one program: a preset of that program, optionally on another host. */
export interface PlanProgram {
  preset: string;
  baseUrl?: string;
}

export interface PlanRegion {
  programs: Partial<Record<AppId, PlanProgram>>;
  /** Where the plan's quota is read with its key; none when the company shows it only in its console. */
  quotaUrl?: string;
  consoleUrl: string;
}

export interface PlanDef {
  id: string;
  name: string;
  regions: Partial<Record<RegionId, PlanRegion>>;
}

export interface Company {
  id: CompanyId;
  name: string;
  icon: string;
  /** Shown in the sidebar before it holds an account. */
  shownByDefault: boolean;
  /** Programs whose own sign-ins belong to this company. */
  logins?: AppId[];
  /** Programs whose relay and gateway accounts this company lists. */
  relayApps?: AppId[];
  plans?: PlanDef[];
  /** Regions whose pay-as-you-go balance can be read with a key. */
  monitorRegions?: RegionId[];
  monitorConsole?: Partial<Record<RegionId, string>>;
}

export const PROGRAM_NAMES: Record<AppId, string> = {
  claude: "Claude Code",
  codex: "Codex",
  gemini: "Gemini CLI",
  kimi: "Kimi Code",
  cursor: "Cursor CLI",
  antigravity: "Antigravity",
  opencode: "OpenCode",
  openclaw: "OpenClaw",
};

export const REGION_NAMES: Record<RegionId, string> = {
  intl: "International",
  cn: "Mainland China",
};

export const COMPANIES: Company[] = [
  {
    id: "anthropic",
    name: "Claude",
    icon: "claude",
    shownByDefault: true,
    logins: ["claude"],
    relayApps: ["claude"],
  },
  {
    id: "openai",
    name: "OpenAI",
    icon: "openai",
    shownByDefault: true,
    logins: ["codex"],
    relayApps: ["codex"],
  },
  {
    id: "google",
    name: "Google",
    icon: "gemini",
    shownByDefault: true,
    logins: ["antigravity", "gemini"],
    relayApps: ["gemini"],
  },
  {
    id: "kimi",
    name: "Kimi",
    icon: "kimi",
    shownByDefault: true,
    logins: ["kimi"],
    relayApps: ["kimi"],
    plans: [
      {
        id: "kimi-code",
        name: "Kimi Code membership",
        regions: {
          intl: {
            programs: {
              claude: { preset: "Kimi Code Plan" },
              opencode: { preset: "Kimi Code Plan" },
            },
            quotaUrl: "https://api.kimi.ai/coding/",
            consoleUrl: "https://www.kimi.ai/code/console",
          },
          cn: {
            programs: {
              claude: {
                preset: "Kimi Code Plan",
                baseUrl: "https://api.kimi.com/coding/",
              },
              opencode: {
                preset: "Kimi Code Plan",
                baseUrl: "https://api.kimi.com/coding/v1",
              },
            },
            quotaUrl: "https://api.kimi.com/coding/",
            consoleUrl: "https://www.kimi.com/code/console",
          },
        },
      },
    ],
    monitorRegions: ["intl", "cn"],
    monitorConsole: {
      intl: "https://platform.moonshot.ai/console",
      cn: "https://platform.moonshot.cn/console",
    },
  },
  {
    id: "cursor",
    name: "Cursor",
    icon: "cursor",
    shownByDefault: true,
    logins: ["cursor"],
  },
  {
    id: "zai",
    name: "Z.ai GLM",
    icon: "zhipu",
    shownByDefault: true,
    plans: [
      {
        id: "coding",
        name: "GLM Coding Plan",
        regions: {
          intl: {
            programs: {
              claude: { preset: "Z.ai GLM Coding Plan" },
              opencode: { preset: "Z.ai GLM Coding Plan" },
            },
            quotaUrl: "https://api.z.ai/api/anthropic",
            consoleUrl: "https://z.ai/manage-apikey/subscription",
          },
          cn: {
            programs: {
              claude: { preset: "Zhipu GLM" },
              opencode: {
                preset: "Zhipu GLM",
                baseUrl: "https://open.bigmodel.cn/api/coding/paas/v4",
              },
            },
            quotaUrl: "https://open.bigmodel.cn/api/anthropic",
            consoleUrl: "https://bigmodel.cn/usercenter/glm-coding/usage",
          },
        },
      },
    ],
  },
  {
    id: "minimax",
    name: "MiniMax",
    icon: "minimax",
    shownByDefault: true,
    plans: [
      {
        id: "m-plan",
        name: "M Plan",
        regions: {
          intl: {
            programs: {
              claude: { preset: "MiniMax M Plan" },
              opencode: { preset: "MiniMax M Plan" },
            },
            quotaUrl: "https://api.minimax.io/anthropic",
            consoleUrl:
              "https://platform.minimax.io/user-center/payment/token-plan",
          },
          cn: {
            programs: {
              claude: { preset: "MiniMax" },
              opencode: {
                preset: "MiniMax M Plan",
                baseUrl: "https://api.minimaxi.com/anthropic/v1",
              },
            },
            quotaUrl: "https://api.minimaxi.com/anthropic",
            consoleUrl:
              "https://platform.minimaxi.com/user-center/payment/coding-plan",
          },
        },
      },
    ],
  },
  {
    id: "deepseek",
    name: "DeepSeek",
    icon: "deepseek",
    shownByDefault: true,
    monitorRegions: ["intl"],
    monitorConsole: { intl: "https://platform.deepseek.com/usage" },
  },
  {
    id: "opencode",
    name: "OpenCode",
    icon: "opencode",
    shownByDefault: true,
    relayApps: ["opencode"],
    plans: [
      {
        id: "go",
        name: "OpenCode Go",
        regions: {
          intl: {
            programs: {
              opencode: { preset: "OpenCode Go" },
              claude: { preset: "OpenCode Go" },
            },
            quotaUrl: "https://opencode.ai/inference/go/anthropic",
            consoleUrl: "https://opencode.ai/auth",
          },
        },
      },
    ],
  },
  {
    id: "commandcode",
    name: "Command Code",
    icon: "commandcode",
    shownByDefault: true,
    plans: [
      {
        id: "plan",
        name: "Command Code plan",
        regions: {
          intl: {
            programs: {
              claude: { preset: "Command Code" },
              opencode: { preset: "Command Code" },
            },
            quotaUrl: "https://api.commandcode.ai/provider",
            consoleUrl: "https://commandcode.ai/studio",
          },
        },
      },
    ],
  },
  {
    id: "alibaba",
    name: "Alibaba Cloud",
    icon: "qwen",
    shownByDefault: false,
    plans: [
      {
        id: "token-plan",
        name: "Token Plan",
        regions: {
          intl: {
            programs: { claude: { preset: "Alibaba Cloud Token Plan" } },
            consoleUrl: "https://modelstudio.console.alibabacloud.com/",
          },
        },
      },
      {
        id: "coding-plan",
        name: "Coding Plan",
        regions: {
          intl: {
            programs: { claude: { preset: "Alibaba Cloud Coding Plan" } },
            consoleUrl: "https://modelstudio.console.alibabacloud.com/",
          },
          cn: {
            programs: { claude: { preset: "Bailian For Coding" } },
            consoleUrl: "https://bailian.console.aliyun.com/",
          },
        },
      },
    ],
  },
  {
    id: "tencent",
    name: "Tencent Cloud",
    icon: "tencent",
    shownByDefault: false,
    plans: [
      {
        id: "token-plan",
        name: "Token Plan",
        regions: {
          intl: {
            programs: { claude: { preset: "Tencent Cloud Token Plan" } },
            consoleUrl: "https://console.tencentcloud.com/tokenhub",
          },
        },
      },
    ],
  },
  {
    id: "byteplus",
    name: "BytePlus",
    icon: "doubao",
    shownByDefault: false,
    plans: [
      {
        id: "coding-plan",
        name: "ModelArk Coding Plan",
        regions: {
          intl: {
            programs: { claude: { preset: "BytePlus ModelArk Coding Plan" } },
            consoleUrl: "https://console.byteplus.com/ark",
          },
          cn: {
            programs: { claude: { preset: "DouBaoSeed" } },
            consoleUrl: "https://console.volcengine.com/ark",
          },
        },
      },
    ],
  },
  {
    id: "stepfun",
    name: "StepFun",
    icon: "stepfun",
    shownByDefault: false,
    plans: [
      {
        id: "step-plan",
        name: "Step Plan",
        regions: {
          cn: {
            programs: { opencode: { preset: "StepFun Step Plan" } },
            consoleUrl: "https://platform.stepfun.com/",
          },
        },
      },
    ],
    monitorRegions: ["intl", "cn"],
    monitorConsole: {
      intl: "https://platform.stepfun.ai/",
      cn: "https://platform.stepfun.com/",
    },
  },
];

export const companyById = (id: string): Company | undefined =>
  COMPANIES.find((company) => company.id === id);

export const planOf = (company: Company | undefined, planId?: string | null) =>
  company?.plans?.find((plan) => plan.id === planId);

/** The programs whose sign-ins are subscription logins of their own company. */
const LOGIN_COMPANY: Partial<Record<AppId, CompanyId>> = {
  claude: "anthropic",
  codex: "openai",
  gemini: "google",
  antigravity: "google",
  kimi: "kimi",
  cursor: "cursor",
};

/** The company whose program a relay or gateway account runs in. */
const RELAY_COMPANY: Partial<Record<AppId, CompanyId>> = {
  claude: "anthropic",
  codex: "openai",
  gemini: "google",
  opencode: "opencode",
  kimi: "kimi",
};

/**
 * Hosts of model makers' own pay-as-you-go APIs. A card on one of these is
 * a pay-as-you-go key run in a program, which no company lists; Switchy
 * only monitors such balances.
 */
const PAY_AS_YOU_GO_HOSTS = [
  "api.deepseek.com",
  "api.moonshot.cn",
  "api.moonshot.ai",
  "open.bigmodel.cn/api/paas",
  "api.z.ai/api/paas",
  "dashscope.aliyuncs.com",
  "dashscope-intl.aliyuncs.com",
  "api.stepfun.ai",
  "api.stepfun.com",
  "api.minimaxi.com/v1",
  "api.minimax.io/v1",
  "api.longcat.chat",
  "api.tbox.cn",
  "api.ant-ling.com",
  "api.xiaomimimo.com",
  "vanchin.streamlake.ai",
  "ark.cn-beijing.volces.com/api/v3",
  "api.kimi.com/v1",
];

/** The endpoint a provider's program is pointed at, if it names one. */
export function providerBaseUrl(appId: AppId, provider: Provider): string {
  const config = (provider.settingsConfig ?? {}) as Record<string, any>;
  switch (appId) {
    case "claude":
      return String(config.env?.ANTHROPIC_BASE_URL ?? "");
    case "gemini":
      return String(config.env?.GOOGLE_GEMINI_BASE_URL ?? "");
    case "opencode":
      return String(config.options?.baseURL ?? "");
    case "openclaw":
      return String(config.baseUrl ?? "");
    case "codex":
      return extractCodexBaseUrl(String(config.config ?? "")) ?? "";
    case "kimi":
      return extractKimiBaseUrl(String(config.config ?? "")) ?? "";
    default:
      return "";
  }
}

/** Whether the provider is the program's own subscription sign-in. */
export function isOwnSignIn(appId: AppId, provider: Provider): boolean {
  const config = (provider.settingsConfig ?? {}) as Record<string, any>;
  switch (appId) {
    case "cursor":
    case "antigravity":
      return true;
    case "claude": {
      const baseUrl = String(config.env?.ANTHROPIC_BASE_URL ?? "")
        .trim()
        .replace(/\/+$/, "");
      return baseUrl === "" || baseUrl === "https://api.anthropic.com";
    }
    case "codex": {
      const key = config.auth?.OPENAI_API_KEY;
      return !key || (typeof key === "string" && key.trim() === "");
    }
    case "kimi":
      return isKimiOfficialConfig(config.config);
    case "gemini": {
      const key = String(config.env?.GEMINI_API_KEY ?? "").trim();
      const baseUrl = String(config.env?.GOOGLE_GEMINI_BASE_URL ?? "").trim();
      return !key && !baseUrl;
    }
    default:
      return false;
  }
}

/**
 * Where a program's provider is listed: under a company, under "other", or
 * nowhere in the sidebar (`null`) — a plan account's program entry, shown
 * on the account's card, or a program setting such as an Oh My OpenCode
 * profile or an OpenClaw provider, shown under Settings → Programs.
 */
export function sidebarEntryOf(
  appId: AppId,
  provider: Provider,
): SidebarEntryId | null {
  if (provider.meta?.accountId) return null;
  if (provider.category === "omo" || provider.category === "omo-slim")
    return null;
  if (appId === "openclaw") return null;
  if (isOwnSignIn(appId, provider)) return LOGIN_COMPANY[appId] ?? null;
  const url = providerBaseUrl(appId, provider).toLowerCase();
  if (PAY_AS_YOU_GO_HOSTS.some((host) => url.includes(host)))
    return OTHER_COMPANY;
  return RELAY_COMPANY[appId] ?? OTHER_COMPANY;
}

/** Relay and gateway presets a company offers for one of its programs. */
const RELAY_PRESETS: Partial<Record<AppId, string[]>> = {
  claude: [
    "PackyCode",
    "Cubence",
    "AIGoCode",
    "RightCode",
    "AICodeMirror",
    "AICoding",
    "CrazyRouter",
    "SSSAiCode",
    "Micu",
    "X-Code API",
    "CTok.ai",
    "OpenRouter",
    "AiHubMix",
    "DMXAPI",
    "SiliconFlow",
    "SiliconFlow en",
    "Novita AI",
    "Nvidia",
    "ModelScope",
    "Compshare",
    "GitHub Copilot",
    "AWS Bedrock (AKSK)",
    "AWS Bedrock (API Key)",
  ],
  codex: [
    "PackyCode",
    "Cubence",
    "AIGoCode",
    "RightCode",
    "AICodeMirror",
    "AICoding",
    "CrazyRouter",
    "SSSAiCode",
    "Micu",
    "X-Code API",
    "CTok.ai",
    "OpenRouter",
    "AiHubMix",
    "DMXAPI",
    "Compshare",
    "Azure OpenAI",
  ],
  gemini: [
    "PackyCode",
    "Cubence",
    "AIGoCode",
    "AICodeMirror",
    "AICoding",
    "CrazyRouter",
    "SSSAiCode",
    "CTok.ai",
    "OpenRouter",
    "Custom",
  ],
  opencode: [
    "PackyCode",
    "Cubence",
    "AIGoCode",
    "RightCode",
    "AICodeMirror",
    "AICoding",
    "CrazyRouter",
    "SSSAiCode",
    "Micu",
    "X-Code API",
    "CTok.ai",
    "OpenRouter",
    "AiHubMix",
    "DMXAPI",
    "Novita AI",
    "Nvidia",
    "ModelScope",
    "AWS Bedrock",
    "OpenAI Compatible",
  ],
  kimi: ["OpenAI Compatible", "Anthropic Compatible"],
};

export const relayPresetNames = (appId: AppId): string[] =>
  RELAY_PRESETS[appId] ?? [];

/** The settings of an account's plan in its region. */
export function planRegion(
  company: Company | undefined,
  planId?: string | null,
  region?: string | null,
): PlanRegion | undefined {
  const plan = planOf(company, planId);
  return plan?.regions[(region as RegionId) ?? "intl"];
}

/** Programs a plan can be used in, in a stable order. */
export function planPrograms(region: PlanRegion | undefined): AppId[] {
  const order: AppId[] = ["claude", "codex", "opencode", "kimi", "gemini"];
  return order.filter((app) => region?.programs[app]);
}

/**
 * A new, empty sign-in card of each program whose provider is its own
 * subscription login: the card's name, icon and empty config. Sign in while
 * the card is in use and the login is filed with it. Written to
 * `signInTemplates.json` for the backend's `add_sign_in_account`.
 */
export const SIGN_IN_TEMPLATES: Partial<
  Record<
    AppId,
    { name: string; icon: string; settingsConfig: Record<string, unknown> }
  >
> = {
  claude: { name: "Claude", icon: "claude", settingsConfig: { env: {} } },
  codex: {
    name: "ChatGPT",
    icon: "openai",
    settingsConfig: { auth: {}, config: "" },
  },
  gemini: { name: "Gemini CLI", icon: "gemini", settingsConfig: { env: {} } },
  kimi: {
    name: "Kimi Code",
    icon: "kimi",
    settingsConfig: { config: KIMI_OFFICIAL_CONFIG, credentials: null },
  },
  cursor: {
    name: "Cursor",
    icon: "cursor",
    settingsConfig: { auth: null, authInfo: null },
  },
  antigravity: {
    name: "Antigravity",
    icon: "google",
    settingsConfig: { credential: null },
  },
};
