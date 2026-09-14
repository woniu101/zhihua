import { invokeNative } from "./nativeBridge";
import type { SceneDurationSeconds } from "../domain/storyboard";

export type LlmProtocol = "openai_chat" | "openai_responses" | "anthropic_messages";
export type LlmProviderId = "deepseek" | "qwen" | "doubao" | "zhipu" | "moonshot" | "minimax" | "openai" | "anthropic" | "gemini" | "custom";

export interface LlmProviderPreset {
  id: LlmProviderId;
  label: string;
  group: "国内服务" | "国际服务" | "自定义";
  protocol: LlmProtocol;
  baseUrl: string;
  model: string;
  modelHint: string;
  help: string;
  protocolLocked: boolean;
}

export const llmProtocolLabels: Record<LlmProtocol, string> = {
  openai_chat: "OpenAI 对话格式",
  openai_responses: "OpenAI Responses 格式",
  anthropic_messages: "Anthropic Messages 格式",
};

export const llmProviderPresets: LlmProviderPreset[] = [
  { id: "deepseek", label: "DeepSeek", group: "国内服务", protocol: "openai_chat", baseUrl: "https://api.deepseek.com", model: "deepseek-chat", modelHint: "deepseek-chat", help: "兼容 OpenAI 对话格式，适合内容提取和分镜规划。", protocolLocked: true },
  { id: "qwen", label: "阿里云百炼 / 通义千问", group: "国内服务", protocol: "openai_chat", baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", model: "qwen-plus", modelHint: "qwen-plus", help: "使用百炼 OpenAI 兼容端点；海外地域可自行修改地址。", protocolLocked: true },
  { id: "doubao", label: "火山方舟 / 豆包", group: "国内服务", protocol: "openai_chat", baseUrl: "https://ark.cn-beijing.volces.com/api/v3", model: "", modelHint: "填写方舟推理接入点 ID", help: "模型字段填写方舟控制台创建的推理接入点 ID。", protocolLocked: true },
  { id: "zhipu", label: "智谱 GLM", group: "国内服务", protocol: "openai_chat", baseUrl: "https://open.bigmodel.cn/api/paas/v4", model: "glm-4-flash", modelHint: "glm-4-flash", help: "使用智谱 OpenAI 兼容接口。", protocolLocked: true },
  { id: "moonshot", label: "Moonshot / Kimi", group: "国内服务", protocol: "openai_chat", baseUrl: "https://api.moonshot.cn/v1", model: "moonshot-v1-8k", modelHint: "moonshot-v1-8k", help: "使用 Moonshot OpenAI 兼容接口。", protocolLocked: true },
  { id: "minimax", label: "MiniMax", group: "国内服务", protocol: "openai_chat", baseUrl: "https://api.minimax.chat/v1", model: "MiniMax-Text-01", modelHint: "MiniMax-Text-01", help: "使用 MiniMax OpenAI 兼容接口。", protocolLocked: true },
  { id: "openai", label: "OpenAI", group: "国际服务", protocol: "openai_responses", baseUrl: "https://api.openai.com/v1", model: "gpt-5-mini", modelHint: "gpt-5-mini", help: "使用 Responses API，适合新项目和结构化输出。", protocolLocked: true },
  { id: "anthropic", label: "Anthropic Claude", group: "国际服务", protocol: "anthropic_messages", baseUrl: "https://api.anthropic.com", model: "claude-sonnet-4-5", modelHint: "claude-sonnet-4-5", help: "使用原生 Messages API，避免兼容层忽略结构化参数。", protocolLocked: true },
  { id: "gemini", label: "Google Gemini", group: "国际服务", protocol: "openai_chat", baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai", model: "gemini-2.5-flash", modelHint: "gemini-2.5-flash", help: "当前使用 Google 的 OpenAI 兼容入口。", protocolLocked: true },
  { id: "custom", label: "自定义服务", group: "自定义", protocol: "openai_chat", baseUrl: "", model: "", modelHint: "填写服务提供的模型名称", help: "选择服务实际支持的协议；地址填写到版本根路径。", protocolLocked: false },
];

export interface LlmConfiguration {
  providerId: LlmProviderId;
  providerLabel: string;
  protocol: LlmProtocol;
  protocolLabel: string;
  baseUrl: string;
  model: string;
  credentialStored: boolean;
  lastVerifiedAt?: string;
}

export interface LlmConnectionTest {
  connected: boolean;
  providerId: LlmProviderId;
  providerLabel: string;
  protocol: LlmProtocol;
  protocolLabel: string;
  model: string;
  structuredOutput: boolean;
  systemInstructions: boolean;
  verifiedAt: string;
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

export interface LlmFailure { code: string; message: string; }

export function normalizeLlmError(error: unknown): LlmFailure {
  if (error && typeof error === "object") {
    const value = error as Partial<LlmFailure>;
    if (typeof value.message === "string") return { code: typeof value.code === "string" ? value.code : "LLM_ERROR", message: value.message };
  }
  return { code: "LLM_ERROR", message: typeof error === "string" ? error : "大模型操作失败" };
}

export const llmRepository = {
  configuration: async () => (await invokeNative<LlmConfiguration>("get_llm_configuration")) ?? {
    providerId: "deepseek" as const,
    providerLabel: "DeepSeek",
    protocol: "openai_chat" as const,
    protocolLabel: llmProtocolLabels.openai_chat,
    baseUrl: "https://api.deepseek.com",
    model: "deepseek-chat",
    credentialStored: false,
  },
  save: async (providerId: LlmProviderId, protocol: LlmProtocol, baseUrl: string, model: string, apiKey: string) =>
    (await invokeNative<LlmConfiguration>("save_llm_configuration", { input: { providerId, protocol, baseUrl, model, apiKey } }))!,
  test: async () => (await invokeNative<LlmConnectionTest>("test_llm_connection"))!,
  clear: () => invokeNative<void>("clear_llm_api_key"),
  reviseScene: async (projectId: string, sceneId: string, instruction: string) =>
    (await invokeNative<SceneRevisionProposal>("storyboard_revise_scene", { input: { projectId, sceneId, instruction } }))!,
};
