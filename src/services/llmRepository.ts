import { invokeNative } from "./nativeBridge";
import type { SceneDurationSeconds } from "../domain/storyboard";

export type LlmProviderId = "deepseek" | "qwen" | "doubao" | "custom";

export interface LlmProviderPreset {
  id: LlmProviderId;
  label: string;
  baseUrl: string;
  model: string;
  modelHint: string;
}

export const llmProviderPresets: LlmProviderPreset[] = [
  { id: "deepseek", label: "DeepSeek", baseUrl: "https://api.deepseek.com", model: "deepseek-chat", modelHint: "deepseek-chat" },
  { id: "qwen", label: "通义千问", baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", model: "qwen-plus", modelHint: "qwen-plus" },
  { id: "doubao", label: "豆包", baseUrl: "https://ark.cn-beijing.volces.com/api/v3", model: "", modelHint: "填写方舟推理接入点 ID" },
  { id: "custom", label: "自定义兼容服务", baseUrl: "", model: "", modelHint: "填写服务提供的模型名称" },
];

export interface LlmConfiguration {
  providerId: LlmProviderId;
  providerLabel: string;
  baseUrl: string;
  model: string;
  credentialStored: boolean;
}

export interface LlmConnectionTest {
  connected: boolean;
  providerId: LlmProviderId;
  providerLabel: string;
  model: string;
}

export interface SceneRevisionProposal {
  title: string;
  purpose: string;
  narration: string;
  onScreenText: string[];
  visualPlan: string;
  ambientSound: string;
  targetDurationSec: SceneDurationSeconds;
  changeSummary: string;
  providerId: LlmProviderId;
  model: string;
  templateVersion: string;
}

export interface LlmFailure {
  code: string;
  message: string;
}

export function normalizeLlmError(error: unknown): LlmFailure {
  if (error && typeof error === "object") {
    const value = error as Partial<LlmFailure>;
    if (typeof value.message === "string") {
      return { code: typeof value.code === "string" ? value.code : "LLM_ERROR", message: value.message };
    }
  }
  return { code: "LLM_ERROR", message: typeof error === "string" ? error : "大模型操作失败" };
}

export const llmRepository = {
  configuration: async () =>
    (await invokeNative<LlmConfiguration>("get_llm_configuration")) ?? {
      providerId: "deepseek" as const,
      providerLabel: "DeepSeek",
      baseUrl: "https://api.deepseek.com",
      model: "deepseek-chat",
      credentialStored: false,
    },
  save: async (providerId: LlmProviderId, baseUrl: string, model: string, apiKey: string) =>
    (await invokeNative<LlmConfiguration>("save_llm_configuration", {
      input: { providerId, baseUrl, model, apiKey },
    }))!,
  test: async () => (await invokeNative<LlmConnectionTest>("test_llm_connection"))!,
  clear: () => invokeNative<void>("clear_llm_api_key"),
  reviseScene: async (projectId: string, sceneId: string, instruction: string) =>
    (await invokeNative<SceneRevisionProposal>("storyboard_revise_scene", {
      input: { projectId, sceneId, instruction },
    }))!,
};
