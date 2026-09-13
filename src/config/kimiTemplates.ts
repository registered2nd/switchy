/**
 * Kimi Code 配置模板
 * 用于新建自定义供应商时的默认配置
 */

export interface KimiTemplate {
  credentials: Record<string, any> | null;
  config: string;
}

/**
 * 获取 Kimi 自定义模板
 *
 * 自定义供应商走 API Key：`[providers.custom]` 加一个模型别名，
 * `default_model` 指向该别名。表单里的 API Key / 请求地址 / 模型名会写回这里。
 */
export function getKimiCustomTemplate(): KimiTemplate {
  const config = `default_model = "custom/gpt-4o"

[providers.custom]
type = "openai"
api_key = ""

[models."custom/gpt-4o"]
provider = "custom"
model = "gpt-4o"`;

  return {
    credentials: null,
    config,
  };
}
