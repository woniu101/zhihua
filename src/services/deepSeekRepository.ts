import { invokeNative } from "./nativeBridge";

export interface DeepSeekConfiguration {
  baseUrl: string;
  model: string;
  credentialStored: boolean;
}

export interface DeepSeekConnectionTest {
  connected: boolean;
  model: string;
}

export interface DeepSeekFailure {
  code: string;
  message: string;
}

export function normalizeDeepSeekError(error: unknown): DeepSeekFailure {
  if (error && typeof error === "object") {
    const value = error as Partial<DeepSeekFailure>;
    if (typeof value.message === "string") {
      return { code: typeof value.code === "string" ? value.code : "DEEPSEEK_ERROR", message: value.message };
    }
  }
  return { code: "DEEPSEEK_ERROR", message: typeof error === "string" ? error : "DeepSeek 操作失败" };
}

export const deepSeekRepository = {
  configuration: async () =>
    (await invokeNative<DeepSeekConfiguration>("get_deepseek_configuration")) ?? {
      baseUrl: "https://api.deepseek.com",
      model: "deepseek-chat",
      credentialStored: false,
    },
  save: async (baseUrl: string, model: string, apiKey: string) =>
    (await invokeNative<DeepSeekConfiguration>("save_deepseek_configuration", {
      input: { baseUrl, model, apiKey },
    }))!,
  test: async () =>
    (await invokeNative<DeepSeekConnectionTest>("test_deepseek_connection"))!,
  clear: () => invokeNative<void>("clear_deepseek_api_key"),
};
